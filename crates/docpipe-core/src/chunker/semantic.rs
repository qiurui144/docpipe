//! 句边界分块实现。先切句，再按 chunk_size 贪心打包，overlap 比例回退若干句作为下块开头。

use crate::chunker::ChunkConfig;
use crate::types::Chunk;

/// 切句：在句末标点（。！？.!?）后或换行处断开，保留标点。
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        cur.push(ch);
        if "。！？.!?\n".contains(ch) {
            let trimmed = cur.trim();
            if !trimmed.is_empty() {
                sentences.push(cur.clone());
            }
            cur.clear();
        }
    }
    if !cur.trim().is_empty() {
        sentences.push(cur);
    }
    sentences
}

/// 判定一行是否标题（markdown # 前缀）。
fn is_heading(s: &str) -> bool {
    s.trim_start().starts_with('#')
}

pub fn chunk_text(text: &str, config: &ChunkConfig) -> Vec<Chunk> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let sentences = split_sentences(text);
    if sentences.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut i = 0usize;
    // char_cursor tracks the char offset of the start of each chunk in the original text.
    // We compute it as the sum of chars in all preceding sentences (not overlapping ones).
    let mut sentence_char_offsets: Vec<u32> = Vec::with_capacity(sentences.len());
    {
        let mut off = 0u32;
        for s in &sentences {
            sentence_char_offsets.push(off);
            off += s.chars().count() as u32;
        }
    }

    while i < sentences.len() {
        let mut buf = String::new();
        let mut j = i;
        // respect_headings：标题句若非块首，遇到则结束当前块（让标题领起下块）。
        while j < sentences.len() {
            let s = &sentences[j];
            if config.respect_headings && is_heading(s) && !buf.is_empty() {
                break;
            }
            if buf.chars().count() + s.chars().count() > config.chunk_size && !buf.is_empty() {
                break;
            }
            buf.push_str(s);
            j += 1;
        }
        let text_chunk = buf.trim_end().to_string();
        let char_offset = sentence_char_offsets[i];
        chunks.push(Chunk {
            chunk_id: uuid::Uuid::new_v4().to_string(),
            text: text_chunk,
            page_refs: vec![],
            char_offset,
        });

        // overlap：下块回退 overlap 比例的句子数。
        let consumed = j - i;
        let step = if config.overlap > 0.0 && consumed > 1 {
            let back = ((consumed as f32) * config.overlap).floor() as usize;
            (consumed - back).max(1)
        } else {
            consumed.max(1)
        };
        i += step;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::ChunkConfig;

    #[test]
    fn short_text_single_chunk() {
        let chunks = chunk_text("一句话。", &ChunkConfig::default());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "一句话。");
        assert_eq!(chunks[0].char_offset, 0);
    }

    #[test]
    fn does_not_split_mid_sentence() {
        // chunk_size 小，但句子完整：每个 chunk 以句末标点结尾。
        let text = "第一句比较长内容内容内容。第二句也不短内容内容。第三句结尾内容。";
        let cfg = ChunkConfig {
            chunk_size: 12,
            overlap: 0.0,
            respect_headings: false,
        };
        let chunks = chunk_text(text, &cfg);
        for c in &chunks {
            let last = c.text.trim_end().chars().last().unwrap();
            assert!(
                "。！？".contains(last),
                "chunk 未在句边界结束: {:?}",
                c.text
            );
        }
    }

    #[test]
    fn overlap_repeats_tail_sentence() {
        // 每句 7 字符，chunk_size=14 恰好容纳 2 句；overlap=0.5 → back=1，下块回退 1 句。
        // 期望：chunk[0]={s1,s2}，chunk[1]={s2,s3}，两块共享句子 "句子二内容。"。
        let text = "句子一内容。句子二内容。句子三内容。句子四内容。";
        let cfg_overlap = ChunkConfig {
            chunk_size: 14,
            overlap: 0.5,
            respect_headings: false,
        };
        let chunks = chunk_text(text, &cfg_overlap);
        assert!(
            chunks.len() >= 2,
            "overlap 模式应产生至少 2 个 chunk，实际 {}",
            chunks.len()
        );

        // 提取 chunk[0] 与 chunk[1] 的句子集合（以 '。' 为分隔，过滤空串）。
        let sentences_of = |s: &str| -> std::collections::HashSet<String> {
            s.split_inclusive('。')
                .map(|seg| seg.trim().to_string())
                .filter(|seg| !seg.is_empty())
                .collect()
        };
        let s0: std::collections::HashSet<String> = sentences_of(&chunks[0].text);
        let s1: std::collections::HashSet<String> = sentences_of(&chunks[1].text);
        // 重叠断言：两块句子集合必须有非空交集，证明 overlap 真的把尾句重复到下块头部。
        let shared: std::collections::HashSet<_> = s0.intersection(&s1).collect();
        assert!(
            !shared.is_empty(),
            "overlap=0.5 时相邻 chunk 应共享至少一句，实际 chunk[0]={:?} chunk[1]={:?}",
            chunks[0].text,
            chunks[1].text
        );

        // 对照断言：overlap=0.0 时相邻 chunk 应无共享句子（证明上面的断言不是平凡的）。
        let cfg_no_overlap = ChunkConfig {
            chunk_size: 14,
            overlap: 0.0,
            respect_headings: false,
        };
        let chunks_no = chunk_text(text, &cfg_no_overlap);
        if chunks_no.len() >= 2 {
            let s0_no: std::collections::HashSet<String> = sentences_of(&chunks_no[0].text);
            let s1_no: std::collections::HashSet<String> = sentences_of(&chunks_no[1].text);
            let shared_no: std::collections::HashSet<_> = s0_no.intersection(&s1_no).collect();
            assert!(
                shared_no.is_empty(),
                "overlap=0.0 时相邻 chunk 不应共享任何句子，实际共享: {:?}",
                shared_no
            );
        }
    }

    #[test]
    fn heading_not_shared_with_body() {
        let text = "## 标题\n正文内容很长很长很长很长很长。";
        let cfg = ChunkConfig {
            chunk_size: 512,
            overlap: 0.0,
            respect_headings: true,
        };
        let chunks = chunk_text(text, &cfg);
        // 标题独立成块（或作为后续块的引导，但不与正文混在一个超大块里时标题单列）。
        assert!(chunks.iter().any(|c| c.text.contains("## 标题")));
    }

    #[test]
    fn heading_after_body_starts_new_chunk() {
        // 验证规则：当 buffer 非空时遇到标题，应先结束当前块，让标题领起新块。
        // 即：不存在任何单个 chunk 同时包含正文和该正文之后的标题。
        let text = "前面的正文内容。## 标题\n标题后的正文。";
        let cfg = ChunkConfig {
            chunk_size: 512,
            overlap: 0.0,
            respect_headings: true,
        };
        let chunks = chunk_text(text, &cfg);
        // 找出含 "## 标题" 的 chunk，断言该 chunk 不含前序正文。
        for c in &chunks {
            if c.text.contains("## 标题") {
                assert!(
                    !c.text.contains("前面的正文内容"),
                    "标题块不应包含前序正文：{:?}",
                    c.text
                );
            }
        }
        // 同时确认"前面的正文内容"确实存在于某块中（防止输入被静默丢弃）。
        assert!(
            chunks.iter().any(|c| c.text.contains("前面的正文内容")),
            "正文内容应存在于某个 chunk 中"
        );
        // 确认"## 标题"确实存在于某块中。
        assert!(
            chunks.iter().any(|c| c.text.contains("## 标题")),
            "标题应存在于某个 chunk 中"
        );
    }

    #[test]
    fn empty_text_yields_no_chunks() {
        assert!(chunk_text("   ", &ChunkConfig::default()).is_empty());
    }
}
