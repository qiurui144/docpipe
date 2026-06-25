//! LLM NER：OpenAI-compat，schema-guided JSON + retry≤3 + 弱模型降级（§4.5）。
use std::time::Duration;
use serde::Deserialize;
use super::{PiiEntity, PiiKind, PiiSource};

#[derive(Debug, Clone)]
pub struct NerConfig { pub base_url: String, pub model: String, pub api_key: Option<String>, pub enabled: bool }

impl NerConfig {
    pub fn from_env() -> Self {
        let base_url = std::env::var("DOCPIPE_PII_BASE_URL").unwrap_or_default();
        Self {
            enabled: !base_url.is_empty(),
            base_url,
            model: std::env::var("DOCPIPE_PII_MODEL").unwrap_or_else(|_| "deepseek-v4".into()),
            api_key: std::env::var("DOCPIPE_PII_API_KEY").ok(),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct RawEnt { pub(crate) kind: String, pub(crate) text: String }

// 把 LLM 返回的 {kind,text} 列表按 text 在原文中定位，产出带 char 偏移的 PiiEntity。
pub(crate) fn locate(text: &str, raw: &[RawEnt]) -> Vec<PiiEntity> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    for r in raw {
        let kind = match r.kind.as_str() {
            "person" => PiiKind::Person, "address" => PiiKind::Address, "org" => PiiKind::Org,
            _ => continue,
        };
        let needle: Vec<char> = r.text.chars().collect();
        if needle.is_empty() { continue; }
        if let Some(start) = (0..=chars.len().saturating_sub(needle.len()))
            .find(|&i| chars[i..i + needle.len()] == needle[..]) {
            out.push(PiiEntity { kind, text: r.text.clone(), start, end: start + needle.len(), confidence: 0.9, source: PiiSource::Llm });
        }
    }
    out
}

pub struct LlmNer { cfg: NerConfig, client: reqwest::Client }

#[derive(Deserialize)]
struct ChatResp { choices: Vec<Choice> }
#[derive(Deserialize)]
struct Choice { message: Msg }
#[derive(Deserialize)]
struct Msg { content: String }
#[derive(Deserialize)]
struct NerPayload { entities: Vec<RawEnt> }

const SYS: &str = "你是 PII 抽取器。只抽取人名(person)/详细地址(address)/组织机构名(org)。\
仅输出 JSON：{\"entities\":[{\"kind\":\"person|address|org\",\"text\":\"原文片段\"}]}。\
text 必须是原文中逐字出现的子串，不得改写。无实体则 entities 为空数组。\
示例1 输入「某甲与某机构公司签约」→ {\"entities\":[{\"kind\":\"person\",\"text\":\"某甲\"},{\"kind\":\"org\",\"text\":\"某机构公司\"}]}。\
示例2 输入「金额 100 元」→ {\"entities\":[]}。";

impl LlmNer {
    pub fn new(cfg: NerConfig) -> Self { Self { cfg, client: reqwest::Client::new() } }

    pub async fn detect(&self, text: &str) -> std::result::Result<Vec<PiiEntity>, String> {
        if !self.cfg.enabled { return Ok(Vec::new()); }
        let body = serde_json::json!({
            "model": self.cfg.model,
            "response_format": {"type": "json_object"},
            "messages": [{"role":"system","content":SYS},{"role":"user","content":text}]
        });
        let mut last = String::new();
        for attempt in 0..3u32 {
            let mut req = self.client.post(format!("{}/chat/completions", self.cfg.base_url.trim_end_matches('/')))
                .timeout(Duration::from_secs(60)).json(&body);
            if let Some(k) = &self.cfg.api_key { req = req.bearer_auth(k); }
            match req.send().await {
                Ok(r) if r.status().is_success() => {
                    match r.json::<ChatResp>().await.ok().and_then(|c| c.choices.into_iter().next()) {
                        Some(ch) => match serde_json::from_str::<NerPayload>(ch.message.content.trim()) {
                            Ok(p) => return Ok(locate(text, &p.entities)),
                            Err(e) => last = format!("bad-json: {e}"),
                        },
                        None => last = "no-choices".into(),
                    }
                }
                Ok(r) => last = format!("status {}", r.status()),
                Err(e) => last = format!("request: {e}"),
            }
            if attempt < 2 { tokio::time::sleep(Duration::from_millis(100 * 2u64.pow(attempt))).await; }
        }
        Err(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locate_maps_char_offsets() {
        let text = "签约方 某机构公司 与某甲";
        let raw = vec![RawEnt { kind: "org".into(), text: "某机构公司".into() }, RawEnt { kind: "person".into(), text: "某甲".into() }];
        let ents = locate(text, &raw);
        assert_eq!(ents.len(), 2);
        let org = ents.iter().find(|e| e.kind == PiiKind::Org).unwrap();
        assert_eq!(&text.chars().skip(org.start).take(org.end-org.start).collect::<String>(), "某机构公司");
    }
    #[test]
    fn disabled_config_when_no_base_url() {
        std::env::remove_var("DOCPIPE_PII_BASE_URL");
        assert!(!NerConfig::from_env().enabled);
    }
}
