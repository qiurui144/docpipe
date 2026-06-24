//! GET /v1/jobs/{job_id} —— 查询异步任务状态。

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;

use crate::error::ApiError;
use crate::jobs::Job;
use crate::state::AppState;
use docpipe_core::error::DocError;

pub async fn get_job(
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
) -> Result<Json<Job>, ApiError> {
    state
        .jobs
        .get(&job_id)
        .map(Json)
        .ok_or(ApiError(DocError::JobNotFound))
}
