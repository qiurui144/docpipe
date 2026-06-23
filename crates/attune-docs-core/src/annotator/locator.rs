//! 标注项构造 + 位置哈希。text_hash = sha256(original_text)，文档被改后 verify 失败 → LocatorDrift。

use sha2::{Digest, Sha256};

use crate::error::{DocError, Result};
use crate::types::{AnnotatableItem, AnnotationSource, BBox, TextLocator};

/// Compute stable sha256 hex hash of text for drift detection.
pub fn text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub struct AnnotateRequest {
    pub doc_id: String,
    pub original_text: String,
    pub content: String,
    pub label: String,
    pub color: String,
    pub page_num: u32,
    pub char_offset: u32,
    pub bbox: Option<BBox>,
    pub source: AnnotationSource,
    pub skill_metadata: Option<serde_json::Value>,
}

pub fn create_item(req: AnnotateRequest) -> AnnotatableItem {
    let hash = text_hash(&req.original_text);
    AnnotatableItem {
        item_id: uuid::Uuid::new_v4().to_string(),
        original_text: req.original_text,
        content: req.content,
        label: req.label,
        color: req.color,
        locator: TextLocator {
            page_num: req.page_num,
            char_offset: req.char_offset,
            bbox: req.bbox,
            text_hash: hash,
        },
        source: req.source,
        skill_metadata: req.skill_metadata,
    }
}

/// Verify current text matches annotated locator hash (detects document drift).
pub fn verify_locator(item: &AnnotatableItem, current_text: &str) -> Result<()> {
    if text_hash(current_text) == item.locator.text_hash {
        Ok(())
    } else {
        Err(DocError::LocatorDrift)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AnnotationSource, BBox};

    #[test]
    fn text_hash_is_stable_and_sensitive() {
        let a = text_hash("跨行汇款");
        let b = text_hash("跨行汇款");
        let c = text_hash("跨行汇款 ");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64); // sha256 hex
    }

    #[test]
    fn create_item_fills_id_and_hash() {
        let req = AnnotateRequest {
            doc_id: "d1".into(),
            original_text: "跨行汇款".into(),
            content: "该笔汇款...".into(),
            label: "关键交易".into(),
            color: "#FF4444".into(),
            page_num: 1,
            char_offset: 240,
            bbox: Some(BBox { x: 1189, y: 350, w: 151, h: 30 }),
            source: AnnotationSource::Ai,
            skill_metadata: None,
        };
        let item = create_item(req);
        assert!(!item.item_id.is_empty());
        assert_eq!(item.locator.text_hash, text_hash("跨行汇款"));
        assert_eq!(item.locator.page_num, 1);
    }

    #[test]
    fn verify_locator_detects_drift() {
        let req = AnnotateRequest {
            doc_id: "d1".into(),
            original_text: "原始文字".into(),
            content: "x".into(),
            label: "l".into(),
            color: "#000000".into(),
            page_num: 1,
            char_offset: 0,
            bbox: None,
            source: AnnotationSource::Human,
            skill_metadata: None,
        };
        let item = create_item(req);
        assert!(verify_locator(&item, "原始文字").is_ok());
        let err = verify_locator(&item, "被修改的文字").unwrap_err();
        assert_eq!(err.code(), "locator-drift");
    }
}
