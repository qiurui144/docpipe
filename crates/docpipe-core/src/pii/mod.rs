//! PII 检测：确定性正则 + LLM NER + 可逆脱敏（spec 2026-06-25-docpipe-pii-detection）。

mod patterns;
pub use patterns::detect_regex;

mod redact;
pub use redact::redact_text;

pub mod llm;
pub use llm::{LlmNer, NerConfig};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiiKind {
    IdCard,
    Phone,
    Email,
    BankCard,
    Plate,
    Ipv4,
    Person,
    Address,
    Org,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiiSource {
    Regex,
    Llm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PiiEntity {
    pub kind: PiiKind,
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub confidence: f32,
    pub source: PiiSource,
}

#[derive(Debug, Clone, Serialize)]
pub struct DetectResult {
    pub entities: Vec<PiiEntity>,
    pub warnings: Vec<String>,
}

fn overlaps(a: &PiiEntity, b: &PiiEntity) -> bool {
    a.start < b.end && b.start < a.end
}

/// 运行正则检测（始终）+ LLM NER（仅 ner.is_some() 时）。
/// LLM 出错时降级：推入 warning 并继续返回正则结果（不 panic，不 5xx）。
/// Regex 实体优先：与任何正则实体重叠的 LLM 实体被丢弃。
/// `types` 过滤输出类别。
pub async fn detect(text: &str, ner: Option<&LlmNer>, types: Option<&[PiiKind]>) -> DetectResult {
    let mut entities = detect_regex(text);
    let mut warnings = Vec::new();
    if let Some(n) = ner {
        match n.detect(text).await {
            Ok(llm_ents) => {
                for e in llm_ents {
                    if !entities.iter().any(|r| r.source == PiiSource::Regex && overlaps(r, &e)) {
                        entities.push(e);
                    }
                }
            }
            Err(reason) => warnings.push(format!(
                "llm-unavailable: {reason}; name/address/org detection skipped"
            )),
        }
    }
    if let Some(t) = types {
        entities.retain(|e| t.contains(&e.kind));
    }
    entities.sort_by_key(|e| e.start);
    DetectResult { entities, warnings }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pii_kind_serializes_snake_case() {
        let j = serde_json::to_string(&PiiKind::IdCard).unwrap();
        assert_eq!(j, "\"id_card\"");
    }

    #[test]
    fn entity_roundtrips() {
        let e = PiiEntity { kind: PiiKind::Email, text: "a@b.co".into(), start: 0, end: 6, confidence: 1.0, source: PiiSource::Regex };
        let j = serde_json::to_string(&e).unwrap();
        let back: PiiEntity = serde_json::from_str(&j).unwrap();
        assert_eq!(e, back);
    }

    #[tokio::test]
    async fn regex_only_when_no_ner() {
        let r = detect("邮箱 a@b.co", None, None).await;
        assert!(r.entities.iter().any(|e| e.kind == PiiKind::Email));
        assert!(r.warnings.is_empty());
    }

    #[tokio::test]
    async fn type_filter_limits_kinds() {
        let r = detect("a@b.co 13800138000", None, Some(&[PiiKind::Phone])).await;
        assert!(!r.entities.is_empty(), "expected at least one Phone entity");
        assert!(r.entities.iter().all(|e| e.kind == PiiKind::Phone));
    }
}
