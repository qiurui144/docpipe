//! 标注层 — AnnotatableItem 创建 + TextLocator 漂移校验（行业无关，spec §3）。

pub mod locator;

pub use locator::{create_item, text_hash, verify_locator, AnnotateRequest};
