//! 极简进程内异步任务队列 —— 内存注册表 + tokio Semaphore 限并发。
//! 无 Redis/心跳/TTL；job 重启即失（v1.1 ephemeral，spec §11）。

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use docpipe_core::error::Result as CoreResult;
use docpipe_core::types::IngestResult;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub job_id: String,
    pub status: JobStatus,
    pub created_at: String,
    pub result: Option<IngestResult>,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct JobQueue {
    jobs: Arc<Mutex<HashMap<String, Job>>>,
    sem: Arc<tokio::sync::Semaphore>,
}

impl JobQueue {
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
            sem: Arc::new(tokio::sync::Semaphore::new(max_concurrency.max(1))),
        }
    }

    /// 提交一个产出 IngestResult 的 future，立即返回 job_id（状态 Queued）。
    pub fn submit<F>(&self, fut: F, created_at: String) -> String
    where
        F: Future<Output = CoreResult<IngestResult>> + Send + 'static,
    {
        let job_id = uuid::Uuid::new_v4().to_string();
        {
            let mut jobs = self.jobs.lock().unwrap();
            jobs.insert(
                job_id.clone(),
                Job {
                    job_id: job_id.clone(),
                    status: JobStatus::Queued,
                    created_at,
                    result: None,
                    error: None,
                },
            );
        }
        let jobs = self.jobs.clone();
        let sem = self.sem.clone();
        let id = job_id.clone();
        tokio::spawn(async move {
            let _permit = sem.acquire().await;
            if let Some(j) = jobs.lock().unwrap().get_mut(&id) {
                j.status = JobStatus::Running;
            }
            let outcome = fut.await;
            let mut g = jobs.lock().unwrap();
            if let Some(j) = g.get_mut(&id) {
                match outcome {
                    Ok(r) => {
                        j.status = JobStatus::Done;
                        j.result = Some(r);
                    }
                    Err(e) => {
                        j.status = JobStatus::Failed;
                        j.error = Some(e.to_string());
                    }
                }
            }
        });
        job_id
    }

    pub fn get(&self, id: &str) -> Option<Job> {
        self.jobs.lock().unwrap().get(id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use docpipe_core::types::{IngestResult, OcrBackendKind};

    fn ok_result() -> IngestResult {
        IngestResult {
            doc_id: "d1".into(),
            collection: "default".into(),
            chunk_count: 1,
            chunk_ids: vec!["d1:a".into()],
            backend: OcrBackendKind::TextLayer,
            ocr_used: false,
        }
    }

    #[tokio::test]
    async fn job_runs_to_done() {
        let q = JobQueue::new(2);
        let id = q.submit(async { Ok(ok_result()) }, "2026-06-24T00:00:00Z".into());
        // poll until done
        for _ in 0..50 {
            if let Some(j) = q.get(&id) {
                if matches!(j.status, JobStatus::Done) {
                    assert_eq!(j.result.unwrap().doc_id, "d1");
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("job did not reach Done");
    }

    #[tokio::test]
    async fn job_failure_is_captured() {
        let q = JobQueue::new(2);
        let id = q.submit(
            async { Err(docpipe_core::error::DocError::ParseEmptyResult) },
            "t".into(),
        );
        for _ in 0..50 {
            if let Some(j) = q.get(&id) {
                if matches!(j.status, JobStatus::Failed) {
                    assert!(j.error.unwrap().contains("parse-empty-result"));
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("job did not reach Failed");
    }

    #[tokio::test]
    async fn get_unknown_job_is_none() {
        assert!(JobQueue::new(1).get("nope").is_none());
    }
}
