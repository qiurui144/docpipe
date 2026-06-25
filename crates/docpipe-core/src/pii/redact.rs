//! 区间脱敏：按 char 偏移从后往前替换，产出可逆 mapping。
use super::{PiiEntity, PiiKind};
use std::collections::BTreeMap;

pub fn redact_text(text: &str, entities: &[PiiEntity]) -> (String, BTreeMap<String, String>) {
    let mut ents: Vec<&PiiEntity> = entities.iter().collect();
    ents.sort_by_key(|e| e.start);
    let mut map = BTreeMap::new();
    // 占位符按出现顺序编号
    let mut tags: Vec<(usize, usize, String)> = Vec::new();
    for (i, e) in ents.iter().enumerate() {
        let tag = format!("[{}_{}]", kind_tag(e.kind), i + 1);
        map.insert(tag.clone(), e.text.clone());
        tags.push((e.start, e.end, tag));
    }
    // 从后往前替换，避免偏移失效
    let chars: Vec<char> = text.chars().collect();
    let mut out = chars.clone();
    for (start, end, tag) in tags.into_iter().rev() {
        out.splice(start..end, tag.chars());
    }
    (out.into_iter().collect(), map)
}

fn kind_tag(k: PiiKind) -> &'static str {
    match k {
        PiiKind::IdCard => "ID_CARD",
        PiiKind::Phone => "PHONE",
        PiiKind::Email => "EMAIL",
        PiiKind::BankCard => "BANK_CARD",
        PiiKind::Plate => "PLATE",
        PiiKind::Ipv4 => "IPV4",
        PiiKind::Person => "PERSON",
        PiiKind::Address => "ADDRESS",
        PiiKind::Org => "ORG",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pii::PiiSource;

    fn ent(kind: PiiKind, t: &str, s: usize, e: usize) -> super::PiiEntity {
        PiiEntity {
            kind,
            text: t.into(),
            start: s,
            end: e,
            confidence: 1.0,
            source: PiiSource::Regex,
        }
    }
    #[test]
    fn redacts_back_to_front_and_is_reversible() {
        let text = "甲 a@b.co 乙 13800138000";
        // a@b.co at chars [2,8), phone at chars [11,22)
        let ents = vec![
            ent(PiiKind::Email, "a@b.co", 2, 8),
            ent(PiiKind::Phone, "13800138000", 11, 22),
        ];
        let (red, map) = redact_text(text, &ents);
        assert!(red.contains("[EMAIL_1]") && red.contains("[PHONE_2]"));
        assert!(!red.contains("a@b.co") && !red.contains("13800138000"));
        assert_eq!(map.get("[EMAIL_1]").unwrap(), "a@b.co");
    }
    #[test]
    fn empty_entities_returns_text_unchanged() {
        let (red, map) = redact_text("hello", &[]);
        assert_eq!(red, "hello");
        assert!(map.is_empty());
    }
}
