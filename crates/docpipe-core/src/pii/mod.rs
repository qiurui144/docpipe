//! PII 检测：确定性正则 + LLM NER + 可逆脱敏（spec 2026-06-25-docpipe-pii-detection）。

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
}
