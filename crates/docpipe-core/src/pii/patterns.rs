//! 确定性 PII 正则 + 校验位（零成本，永远可用）。
use once_cell::sync::Lazy;
use regex::Regex;
use super::{PiiEntity, PiiKind, PiiSource};

static RE_EMAIL: Lazy<Regex> = Lazy::new(|| Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap());
static RE_PHONE: Lazy<Regex> = Lazy::new(|| Regex::new(r"1[3-9]\d{9}").unwrap());
static RE_IPV4:  Lazy<Regex> = Lazy::new(|| Regex::new(r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)\b").unwrap());
static RE_ID:    Lazy<Regex> = Lazy::new(|| Regex::new(r"[1-9]\d{16}[\dXx]").unwrap());
static RE_BANK:  Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{13,19}\b").unwrap());
static RE_PLATE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[京津沪渝冀豫云辽黑湘皖鲁新苏浙赣鄂桂甘晋蒙陕吉闽贵粤青藏川宁琼][A-HJ-NP-Z][A-HJ-NP-Z0-9]{5}").unwrap());

pub fn luhn_ok(digits: &str) -> bool {
    let ds: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
    if ds.len() < 13 { return false; }
    let mut sum = 0u32;
    for (i, &d) in ds.iter().rev().enumerate() {
        if i % 2 == 1 { let dd = d * 2; sum += if dd > 9 { dd - 9 } else { dd }; }
        else { sum += d; }
    }
    sum % 10 == 0
}

pub fn is_valid_cn_id(s: &str) -> bool {
    let c: Vec<char> = s.chars().collect();
    if c.len() != 18 || !c[..17].iter().all(|x| x.is_ascii_digit()) { return false; }
    const W: [u32; 17] = [7,9,10,5,8,4,2,1,6,3,7,9,10,5,8,4,2];
    const CHK: [char; 11] = ['1','0','X','9','8','7','6','5','4','3','2'];
    let sum: u32 = c[..17].iter().zip(W).map(|(ch, w)| ch.to_digit(10).unwrap() * w).sum();
    CHK[(sum % 11) as usize] == c[17].to_ascii_uppercase()
}

// 字节偏移 → 字符偏移
fn byte_to_char(text: &str, byte_idx: usize) -> usize {
    text[..byte_idx].chars().count()
}

fn push_matches(text: &str, re: &Regex, kind: PiiKind, out: &mut Vec<PiiEntity>, filter: impl Fn(&str) -> bool) {
    for m in re.find_iter(text) {
        if !filter(m.as_str()) { continue; }
        out.push(PiiEntity {
            kind,
            text: m.as_str().to_string(),
            start: byte_to_char(text, m.start()),
            end:   byte_to_char(text, m.end()),
            confidence: 1.0,
            source: PiiSource::Regex,
        });
    }
}

pub fn detect_regex(text: &str) -> Vec<PiiEntity> {
    let mut out = Vec::new();
    push_matches(text, &RE_EMAIL, PiiKind::Email,    &mut out, |_| true);
    push_matches(text, &RE_PHONE, PiiKind::Phone,    &mut out, |_| true);
    push_matches(text, &RE_IPV4,  PiiKind::Ipv4,     &mut out, |_| true);
    push_matches(text, &RE_ID,    PiiKind::IdCard,   &mut out, is_valid_cn_id);
    push_matches(text, &RE_BANK,  PiiKind::BankCard, &mut out, |s| !is_valid_cn_id(s) && luhn_ok(s));
    push_matches(text, &RE_PLATE, PiiKind::Plate,    &mut out, |_| true);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<PiiKind> {
        let mut k: Vec<_> = detect_regex(text).into_iter().map(|e| e.kind).collect();
        k.sort_by_key(|x| format!("{x:?}")); k
    }

    #[test]
    fn email_detected_with_char_offsets() {
        let text = "联系 test-user@example.com 谢谢"; // 中文前缀确保 char≠byte
        let e: Vec<_> = detect_regex(text).into_iter().filter(|e| e.kind == PiiKind::Email).collect();
        assert_eq!(e.len(), 1);
        assert_eq!(&text.chars().skip(e[0].start).take(e[0].end - e[0].start).collect::<String>(), "test-user@example.com");
    }

    #[test]
    fn cn_id_checksum_rejects_invalid() {
        assert!(is_valid_cn_id("11010519491231002X"));   // synthetic, valid checksum
        assert!(!is_valid_cn_id("110105194912310021")); // wrong check digit
        let e: Vec<_> = detect_regex("证件 110105194912310021 末位错").into_iter().filter(|e| e.kind == PiiKind::IdCard).collect();
        assert!(e.is_empty(), "invalid checksum must not match");
    }

    #[test]
    fn bank_card_luhn() {
        assert!(luhn_ok("4242424242424242"));
        assert!(!luhn_ok("4242424242424241"));
    }

    #[test]
    fn phone_and_ipv4_and_plate() {
        assert!(kinds("手机 13800138000").contains(&PiiKind::Phone));
        assert!(kinds("网关 192.168.1.1 内网").contains(&PiiKind::Ipv4));
        assert!(kinds("车牌 京A12345").contains(&PiiKind::Plate));
    }

    #[test]
    fn empty_text_no_matches() {
        assert!(detect_regex("").is_empty());
    }
}
