//! PII NER 评测工具 — 多种子 F1 测量（§4.5D 三档模型矩阵）。
//!
//! 评测调用真实生产路径 `docpipe_core::pii::detect`，不使用 mock。
//! 必须在指定目标机器上运行（§1.6），不在开发主机执行。
//!
//! 用法：
//!   cargo run --release --example pii_ner_eval -- [--corpus <path>] [--seeds <N>]
//!
//! 所需环境变量：
//!   DOCPIPE_PII_BASE_URL  — OpenAI-compatible NER 端点（必填）
//!   DOCPIPE_PII_MODEL     — 模型名（默认 deepseek-v4）
//!   DOCPIPE_PII_API_KEY   — Bearer token（可选）

use docpipe_core::pii::{detect, LlmNer, NerConfig, PiiKind};
use serde::Deserialize;
use std::{collections::HashMap, path::PathBuf};

// ── 语料结构 ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GoldEntity {
    kind: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct CorpusCase {
    text: String,
    entities: Vec<GoldEntity>,
}

// ── 评测指标 ──────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct Counts {
    tp: usize,
    fp: usize,
    r#fn: usize,
}

impl Counts {
    fn precision(&self) -> f64 {
        let denom = (self.tp + self.fp) as f64;
        if denom == 0.0 {
            0.0
        } else {
            self.tp as f64 / denom
        }
    }

    fn recall(&self) -> f64 {
        let denom = (self.tp + self.r#fn) as f64;
        if denom == 0.0 {
            0.0
        } else {
            self.tp as f64 / denom
        }
    }

    fn f1(&self) -> f64 {
        let p = self.precision();
        let r = self.recall();
        let denom = p + r;
        if denom == 0.0 {
            0.0
        } else {
            2.0 * p * r / denom
        }
    }
}

// ── 辅助：kind 字符串 → PiiKind（仅评测范围内的三类）─────────────────────────

fn kind_str(k: &PiiKind) -> Option<&'static str> {
    match k {
        PiiKind::Person => Some("person"),
        PiiKind::Address => Some("address"),
        PiiKind::Org => Some("org"),
        _ => None,
    }
}

fn parse_gold_kind(s: &str) -> Option<PiiKind> {
    match s {
        "person" => Some(PiiKind::Person),
        "address" => Some(PiiKind::Address),
        "org" => Some(PiiKind::Org),
        _ => None,
    }
}

// ── 主逻辑 ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // ── 参数解析（手动，不依赖 clap）──────────────────────────────────────────
    let args: Vec<String> = std::env::args().collect();
    let mut corpus_path = PathBuf::from("crates/docpipe-core/tests/fixtures/pii_eval_corpus.jsonl");
    let mut seeds: u32 = 3;

    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--corpus" => {
                i += 1;
                if i < args.len() {
                    corpus_path = PathBuf::from(&args[i]);
                } else {
                    eprintln!("error: --corpus requires a path argument");
                    std::process::exit(1);
                }
            }
            "--seeds" => {
                i += 1;
                if i < args.len() {
                    seeds = args[i].parse().unwrap_or_else(|_| {
                        eprintln!("error: --seeds must be a positive integer");
                        std::process::exit(1);
                    });
                    if seeds == 0 {
                        eprintln!("error: --seeds must be >= 1");
                        std::process::exit(1);
                    }
                } else {
                    eprintln!("error: --seeds requires an integer argument");
                    std::process::exit(1);
                }
            }
            other => {
                eprintln!("error: unknown argument: {other}");
                std::process::exit(1);
            }
        }
        i += 1;
    }

    // ── 检查 NerConfig（no-empty-PASS 纪律）──────────────────────────────────
    let cfg = NerConfig::from_env();
    if !cfg.enabled {
        eprintln!(
            "error: DOCPIPE_PII_BASE_URL not set — \
             eval requires a live OpenAI-compat endpoint. \
             Set DOCPIPE_PII_BASE_URL and re-run."
        );
        std::process::exit(2);
    }

    // ── 提取 endpoint host（不含 credentials / 路径，安全输出）─────────────
    let endpoint_host = {
        let url = cfg.base_url.trim_end_matches('/');
        // 解析出 host，去掉路径和认证信息
        if let Some((_, after_scheme)) = url.split_once("://") {
            after_scheme
                .split('/')
                .next()
                .unwrap_or(after_scheme)
                .to_string()
        } else {
            url.to_string()
        }
    };
    let model_name = cfg.model.clone();

    let ner = LlmNer::new(cfg);

    // ── 加载语料 ──────────────────────────────────────────────────────────────
    let raw = std::fs::read_to_string(&corpus_path).unwrap_or_else(|e| {
        eprintln!("error: cannot read corpus {:?}: {e}", corpus_path);
        std::process::exit(1);
    });

    let cases: Vec<CorpusCase> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(idx, line)| {
            serde_json::from_str(line).unwrap_or_else(|e| {
                eprintln!("error: corpus line {}: {e}", idx + 1);
                std::process::exit(1);
            })
        })
        .collect();

    let n_cases = cases.len();
    if n_cases == 0 {
        eprintln!("error: corpus is empty");
        std::process::exit(1);
    }

    // 评测只关心这三类
    let eval_kinds = [PiiKind::Person, PiiKind::Address, PiiKind::Org];

    // ── 多种子评测 ────────────────────────────────────────────────────────────
    let mut per_seed: Vec<HashMap<&'static str, f64>> = Vec::new();

    for seed in 1..=seeds {
        let mut totals = Counts::default();

        for case in &cases {
            // 金标：(kind_str, normalized_text)
            let gold_set: std::collections::HashSet<(String, String)> = case
                .entities
                .iter()
                .filter_map(|e| {
                    parse_gold_kind(&e.kind).map(|_| (e.kind.clone(), e.text.trim().to_string()))
                })
                .collect();

            // 预测
            let result = detect(&case.text, Some(&ner), Some(&eval_kinds)).await;

            let pred_set: std::collections::HashSet<(String, String)> = result
                .entities
                .iter()
                .filter_map(|e| {
                    kind_str(&e.kind).map(|k| (k.to_string(), e.text.trim().to_string()))
                })
                .collect();

            let tp = pred_set.intersection(&gold_set).count();
            let fp = pred_set.difference(&gold_set).count();
            let r#fn = gold_set.difference(&pred_set).count();

            totals.tp += tp;
            totals.fp += fp;
            totals.r#fn += r#fn;
        }

        let mut seed_map = HashMap::new();
        seed_map.insert("seed", seed as f64);
        seed_map.insert("precision", totals.precision());
        seed_map.insert("recall", totals.recall());
        seed_map.insert("f1", totals.f1());
        per_seed.push(seed_map);

        // 进度提示（stderr，不污染 stdout JSON）
        eprintln!(
            "[seed {seed}/{seeds}] precision={:.4} recall={:.4} f1={:.4}",
            totals.precision(),
            totals.recall(),
            totals.f1()
        );
    }

    // ── 聚合统计 ──────────────────────────────────────────────────────────────
    let f1_values: Vec<f64> = per_seed.iter().map(|m| m["f1"]).collect();
    let p_values: Vec<f64> = per_seed.iter().map(|m| m["precision"]).collect();
    let r_values: Vec<f64> = per_seed.iter().map(|m| m["recall"]).collect();

    fn mean(v: &[f64]) -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        v.iter().sum::<f64>() / v.len() as f64
    }

    fn pop_std(v: &[f64]) -> f64 {
        if v.len() < 2 {
            return 0.0;
        }
        let m = mean(v);
        (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
    }

    // ── 输出 JSON 报告（stdout）───────────────────────────────────────────────
    // 手动构造，避免 serde 问题；不输出 API key 或完整 URL（§1.4）。
    let per_seed_json: Vec<String> = per_seed
        .iter()
        .map(|m| {
            format!(
                r#"{{"seed":{},"precision":{:.6},"recall":{:.6},"f1":{:.6}}}"#,
                m["seed"] as u32, m["precision"], m["recall"], m["f1"]
            )
        })
        .collect();

    let report = format!(
        r#"{{
  "schema_version": 1,
  "harness": "pii_ner_eval",
  "harness_version": "1.0.0",
  "model": {model_json},
  "endpoint_host": {host_json},
  "provenance": "synthetic",
  "n_cases": {n_cases},
  "seeds": {seeds},
  "per_seed": [{per_seed_arr}],
  "f1_mean": {f1_mean:.6},
  "f1_std": {f1_std:.6},
  "precision_mean": {p_mean:.6},
  "recall_mean": {r_mean:.6}
}}"#,
        model_json = serde_json::to_string(&model_name).unwrap(),
        host_json = serde_json::to_string(&endpoint_host).unwrap(),
        n_cases = n_cases,
        seeds = seeds,
        per_seed_arr = per_seed_json.join(", "),
        f1_mean = mean(&f1_values),
        f1_std = pop_std(&f1_values),
        p_mean = mean(&p_values),
        r_mean = mean(&r_values),
    );

    println!("{report}");
}
