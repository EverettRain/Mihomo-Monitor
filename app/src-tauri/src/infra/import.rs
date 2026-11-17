//! Reading host-group entries from the formats people already have them in.
//!
//! - text: one entry per line (domains, addresses, ranges, URLs, Clash rule lines);
//! - CSV / TSV: columns for the group, the entry and optionally its kind, with or without a
//!   header row;
//! - JSON: a list of entries, `{ "group": [entries] }`, a list of objects, a sing-box rule-set,
//!   or this app's own export;
//! - YAML: a Clash rule-provider (`payload:`), or `group: [entries]`.
//!
//! Every line or item that cannot be read is reported with the reason instead of failing the
//! whole import.

use crate::domain::hostgroup::{parse_entry, Rule, RuleKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use yaml_rust2::{Yaml, YamlLoader};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Auto,
    Text,
    Csv,
    Json,
    Yaml,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// The group the source names; `None` goes to the group chosen for the import.
    pub group: Option<String>,
    #[serde(flatten)]
    pub rule: Rule,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// 1-based line (text, CSV, YAML) or item (JSON) number; 0 when there is none.
    pub at: usize,
    pub text: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Parsed {
    /// The format read (what `Auto` detected).
    pub format: Format,
    pub entries: Vec<Entry>,
    pub problems: Vec<Problem>,
}

const GROUP_FIELDS: [&str; 9] = ["group", "name", "service", "category", "分组", "组", "组名", "服务", "名称"];
const VALUE_FIELDS: [&str; 14] =
    ["value", "domain", "host", "ip", "rule", "entry", "pattern", "address", "域名", "地址", "值", "条目", "规则", "内容"];
const KIND_FIELDS: [&str; 6] = ["type", "kind", "match", "类型", "方式", "匹配"];
const LIST_FIELDS: [&str; 9] = ["entries", "rules", "domains", "items", "values", "list", "payload", "条目", "规则"];

fn is_field(name: &str, fields: &[&str]) -> bool {
    let n = name.trim().to_lowercase();
    fields.contains(&n.as_str())
}

fn content_lines(text: &str) -> Vec<(usize, &str)> {
    text.lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim()))
        .filter(|(_, l)| !l.is_empty() && !l.starts_with('#') && !l.starts_with("//"))
        .collect()
}

fn detect(text: &str) -> Format {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with('{') || t.starts_with('[') {
        return Format::Json;
    }
    let lines = content_lines(t);
    if lines.iter().any(|(_, l)| l.starts_with("payload:")) {
        return Format::Yaml;
    }
    let with_columns = lines.iter().filter(|(_, l)| l.contains(',') || l.contains('\t')).count();
    if with_columns * 2 <= lines.len() {
        return Format::Text;
    }
    // Comma lines that read as Clash rules are text; anything else with columns is a table.
    let rules = lines.iter().filter(|(_, l)| l.contains(',') && matches!(parse_entry(l, RuleKind::Suffix), Ok(Some(_)))).count();
    if rules * 5 >= with_columns * 4 {
        Format::Text
    } else {
        Format::Csv
    }
}

pub fn parse(text: &str, format: Format) -> Parsed {
    let text = text.trim_start_matches('\u{feff}');
    let format = if format == Format::Auto { detect(text) } else { format };
    let mut out = Parsed { format, entries: Vec::new(), problems: Vec::new() };
    match format {
        Format::Auto | Format::Text => text_entries(text, &mut out),
        Format::Csv => csv_entries(text, &mut out),
        Format::Json => json_entries(text, &mut out),
        Format::Yaml => yaml_entries(text, &mut out),
    }
    let mut seen = std::collections::HashSet::new();
    out.entries.retain(|e| seen.insert((e.group.clone(), e.rule.clone())));
    out
}

fn push(out: &mut Parsed, at: usize, text: &str, group: Option<String>, read: Result<Option<Rule>, String>) {
    match read {
        Ok(Some(rule)) => out.entries.push(Entry { group: group.filter(|g| !g.trim().is_empty()).map(|g| g.trim().to_string()), rule }),
        Ok(None) => {}
        Err(reason) => out.problems.push(Problem { at, text: text.chars().take(120).collect(), reason }),
    }
}

fn text_entries(text: &str, out: &mut Parsed) {
    for (at, line) in text.lines().enumerate() {
        push(out, at + 1, line, None, parse_entry(line, RuleKind::Suffix));
    }
}

/// An entry with its kind given separately, or read as people write it.
fn entry_with_kind(value: &str, kind: Option<&str>, bare: RuleKind) -> Result<Option<Rule>, String> {
    match kind.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => match RuleKind::parse(k) {
            Some(kind) => Rule::new(kind, value).map(Some),
            None => Err(format!("不认识的类型“{k}”")),
        },
        None => parse_entry(value, bare),
    }
}

fn csv_entries(text: &str, out: &mut Parsed) {
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let delimiter = if first.contains('\t') && !first.contains(',') { b'\t' } else { b',' };
    let mut reader =
        csv::ReaderBuilder::new().has_headers(false).flexible(true).trim(csv::Trim::All).delimiter(delimiter).from_reader(text.as_bytes());
    let mut columns: Option<(Option<usize>, usize, Option<usize>)> = None;
    for (i, record) in reader.records().enumerate() {
        let at = i + 1;
        let record = match record {
            Ok(r) => r,
            Err(e) => {
                out.problems.push(Problem { at, text: String::new(), reason: format!("CSV 格式错误：{e}") });
                continue;
            }
        };
        let cells: Vec<&str> = record.iter().collect();
        if cells.iter().all(|c| c.is_empty()) || cells[0].starts_with('#') {
            continue;
        }
        if i == 0 && cells.iter().any(|c| is_field(c, &VALUE_FIELDS)) {
            let find = |fields: &[&str]| cells.iter().position(|c| is_field(c, fields));
            if let Some(value) = find(&VALUE_FIELDS) {
                columns = Some((find(&GROUP_FIELDS), value, find(&KIND_FIELDS)));
                continue;
            }
        }
        let line = cells.join(",");
        let (group, value, kind) = match columns {
            Some((g, v, k)) => {
                (g.and_then(|g| cells.get(g)).copied(), cells.get(v).copied().unwrap_or(""), k.and_then(|k| cells.get(k)).copied())
            }
            None => guess_columns(&cells),
        };
        push(out, at, &line, group.map(str::to_string), entry_with_kind(value, kind, RuleKind::Suffix));
    }
}

/// Without a header: a cell naming a kind is the kind, the last cell that reads as an entry is
/// the entry, and the first remaining cell is the group.
fn guess_columns<'a>(cells: &[&'a str]) -> (Option<&'a str>, &'a str, Option<&'a str>) {
    if cells.len() == 1 {
        return (None, cells[0], None);
    }
    let kind = cells.iter().position(|c| RuleKind::parse(c).is_some());
    let value = (0..cells.len()).rev().find(|i| Some(*i) != kind && matches!(parse_entry(cells[*i], RuleKind::Suffix), Ok(Some(_))));
    let value = value.unwrap_or(cells.len() - 1);
    let group = (0..cells.len()).find(|i| *i != value && Some(*i) != kind && !cells[*i].is_empty());
    (group.map(|g| cells[g]), cells[value], kind.map(|k| cells[k]))
}

fn json_text(v: &Value) -> String {
    let s = v.to_string();
    s.chars().take(120).collect()
}

fn field<'a>(obj: &'a serde_json::Map<String, Value>, fields: &[&str]) -> Option<&'a Value> {
    obj.iter().find(|(k, _)| is_field(k, fields)).map(|(_, v)| v)
}

fn json_entries(text: &str, out: &mut Parsed) {
    let root: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            out.problems.push(Problem { at: e.line(), text: String::new(), reason: format!("JSON 格式错误：{e}") });
            return;
        }
    };
    let mut n = 0;
    match &root {
        Value::Object(obj) if obj.contains_key("rules") && obj.contains_key("version") => sing_box(obj, out, &mut n),
        Value::Object(obj) if obj.get("groups").is_some_and(Value::is_array) => {
            json_list(obj["groups"].as_array().unwrap_or(&Vec::new()), None, out, &mut n)
        }
        Value::Object(obj) => {
            for (group, items) in obj {
                json_items(items, Some(group.clone()), out, &mut n);
            }
        }
        Value::Array(list) => json_list(list, None, out, &mut n),
        other => out.problems.push(Problem { at: 0, text: json_text(other), reason: "无法识别的 JSON 结构".into() }),
    }
}

fn json_items(items: &Value, group: Option<String>, out: &mut Parsed, n: &mut usize) {
    match items {
        Value::Array(list) => json_list(list, group, out, n),
        other => json_list(std::slice::from_ref(other), group, out, n),
    }
}

fn json_list(list: &[Value], group: Option<String>, out: &mut Parsed, n: &mut usize) {
    for item in list {
        *n += 1;
        match item {
            Value::String(s) => push(out, *n, s, group.clone(), parse_entry(s, RuleKind::Suffix)),
            Value::Object(obj) => {
                let name = field(obj, &GROUP_FIELDS).and_then(Value::as_str).map(str::to_string).or_else(|| group.clone());
                if let Some(items) = field(obj, &LIST_FIELDS).filter(|v| v.is_array()) {
                    json_items(items, name, out, n);
                } else if let Some(value) = field(obj, &VALUE_FIELDS).and_then(Value::as_str) {
                    let kind = field(obj, &KIND_FIELDS).and_then(Value::as_str);
                    push(out, *n, &json_text(item), name, entry_with_kind(value, kind, RuleKind::Suffix));
                } else {
                    out.problems.push(Problem {
                        at: *n,
                        text: json_text(item),
                        reason: "找不到条目字段（如 value、domain、rules）".into(),
                    });
                }
            }
            other => out.problems.push(Problem { at: *n, text: json_text(other), reason: "条目应为字符串或对象".into() }),
        }
    }
}

/// A sing-box rule-set: `{"version": n, "rules": [{"domain": [...], "domain_suffix": [...], ...}]}`.
fn sing_box(obj: &serde_json::Map<String, Value>, out: &mut Parsed, n: &mut usize) {
    let kinds = [
        ("domain", RuleKind::Domain),
        ("domain_suffix", RuleKind::Suffix),
        ("domain_keyword", RuleKind::Keyword),
        ("ip_cidr", RuleKind::Cidr),
    ];
    for rule in obj["rules"].as_array().into_iter().flatten() {
        let Some(rule) = rule.as_object() else { continue };
        for (key, values) in rule {
            let values: Vec<&Value> = match values {
                Value::Array(a) => a.iter().collect(),
                v => vec![v],
            };
            match kinds.iter().find(|(k, _)| k == key) {
                Some((_, kind)) => {
                    for v in values {
                        *n += 1;
                        let s = v.as_str().unwrap_or_default();
                        // sing-box suffixes may start with a dot; the kind already says "with subdomains".
                        push(out, *n, s, None, Rule::new(*kind, s.trim_start_matches('.')).map(Some));
                    }
                }
                None => {
                    *n += 1;
                    out.problems.push(Problem { at: *n, text: key.clone(), reason: format!("不支持 sing-box 的 {key} 规则") });
                }
            }
        }
    }
}

fn yaml_entries(text: &str, out: &mut Parsed) {
    let docs = match YamlLoader::load_from_str(text) {
        Ok(d) => d,
        Err(e) => {
            out.problems.push(Problem { at: e.marker().line() + 1, text: String::new(), reason: format!("YAML 格式错误：{e}") });
            return;
        }
    };
    let Some(doc) = docs.first() else { return };
    // Item numbers count list items; YAML gives no line numbers once loaded.
    let mut n = 0;
    let mut list = |items: &Yaml, group: Option<String>, out: &mut Parsed| {
        for item in items.as_vec().map(Vec::as_slice).unwrap_or(std::slice::from_ref(items)) {
            n += 1;
            match item.as_str() {
                // In a Clash domain set a bare domain is that domain only; `+.` adds subdomains.
                Some(s) => push(out, n, s, group.clone(), parse_entry(s, RuleKind::Domain)),
                None => out.problems.push(Problem { at: n, text: format!("{item:?}"), reason: "条目应为字符串".into() }),
            }
        }
    };
    let key = |k: &str| matches!(doc, Yaml::Hash(h) if h.contains_key(&Yaml::String(k.into())));
    if key("attribution") || (key("groups") && key("version")) {
        out.problems.push(Problem {
            at: 0,
            text: String::new(),
            reason: "这是规则配置文件（分组与归属规则），请用「配置文件」导入".into(),
        });
        return;
    }
    match doc {
        Yaml::Hash(h) if h.contains_key(&Yaml::String("payload".into())) => list(&doc["payload"], None, out),
        Yaml::Hash(h) => {
            for (k, v) in h {
                list(v, k.as_str().map(str::to_string), out);
            }
        }
        Yaml::Array(_) => list(doc, None, out),
        _ => out.problems.push(Problem { at: 0, text: String::new(), reason: "无法识别的 YAML 结构".into() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(p: &Parsed) -> Vec<(Option<&str>, RuleKind, &str)> {
        p.entries.iter().map(|e| (e.group.as_deref(), e.rule.kind, e.rule.value.as_str())).collect()
    }

    #[test]
    fn plain_text_is_one_entry_per_line_with_problems_reported() {
        // Arrange
        let text = "youtube.com\n\n# 注释\n+.googlevideo.com\n149.154.167.51\nnot a domain\nyoutube.com\n";

        // Act
        let p = parse(text, Format::Auto);

        // Assert
        assert_eq!(p.format, Format::Text);
        assert_eq!(
            rules(&p),
            [
                (None, RuleKind::Suffix, "youtube.com"),
                (None, RuleKind::Suffix, "googlevideo.com"),
                (None, RuleKind::Cidr, "149.154.167.51")
            ]
        );
        assert_eq!(p.problems.len(), 1);
        assert_eq!(p.problems[0].at, 6);
    }

    #[test]
    fn clash_rule_lines_are_text_not_csv() {
        // Act
        let p = parse("DOMAIN-SUFFIX,netflix.com,Proxy\nIP-CIDR,23.246.0.0/18,Proxy,no-resolve\n", Format::Auto);

        // Assert
        assert_eq!(p.format, Format::Text);
        assert_eq!(rules(&p), [(None, RuleKind::Suffix, "netflix.com"), (None, RuleKind::Cidr, "23.246.0.0/18")]);
    }

    #[test]
    fn csv_with_a_header_maps_columns_by_name() {
        // Arrange
        let text = "域名,分组,类型\nyoutube.com,YouTube,suffix\nwww.google.com,Google,domain\n\"t.me\",Telegram,\n";

        // Act
        let p = parse(text, Format::Auto);

        // Assert
        assert_eq!(p.format, Format::Csv);
        assert_eq!(
            rules(&p),
            [
                (Some("YouTube"), RuleKind::Suffix, "youtube.com"),
                (Some("Google"), RuleKind::Domain, "www.google.com"),
                (Some("Telegram"), RuleKind::Suffix, "t.me")
            ]
        );
    }

    #[test]
    fn csv_without_a_header_finds_the_entry_column() {
        // Act
        let p = parse("YouTube,youtube.com\nTelegram,91.108.4.0/22\nTelegram\t?\n", Format::Csv);

        // Assert
        assert_eq!(rules(&p), [(Some("YouTube"), RuleKind::Suffix, "youtube.com"), (Some("Telegram"), RuleKind::Cidr, "91.108.4.0/22")]);
        assert_eq!(p.problems.len(), 1);
    }

    #[test]
    fn json_in_its_common_shapes() {
        // Arrange
        let list = r#"["youtube.com", "1.1.1.1"]"#;
        let map = r#"{"YouTube": ["youtube.com", "+.ytimg.com"], "Cloudflare": "1.1.1.1"}"#;
        let objects =
            r#"[{"group": "GitHub", "domain": "github.com"}, {"name": "Steam", "rules": [{"type": "keyword", "value": "steam"}]}]"#;
        let export = r#"{"groups": [{"name": "Apple", "note": "", "rules": [{"kind": "suffix", "value": "apple.com"}]}]}"#;

        // Act
        let got = [list, map, objects, export].map(|t| parse(t, Format::Auto));

        // Assert
        assert!(got.iter().all(|p| p.format == Format::Json && p.problems.is_empty()), "{got:?}");
        assert_eq!(rules(&got[0]), [(None, RuleKind::Suffix, "youtube.com"), (None, RuleKind::Cidr, "1.1.1.1")]);
        assert_eq!(rules(&got[1]).len(), 3);
        assert!(rules(&got[1]).contains(&(Some("Cloudflare"), RuleKind::Cidr, "1.1.1.1")));
        assert_eq!(rules(&got[2]), [(Some("GitHub"), RuleKind::Suffix, "github.com"), (Some("Steam"), RuleKind::Keyword, "steam")]);
        assert_eq!(rules(&got[3]), [(Some("Apple"), RuleKind::Suffix, "apple.com")]);
    }

    #[test]
    fn sing_box_rule_sets() {
        // Arrange
        let text = r#"{"version": 2, "rules": [{"domain": ["a.com"], "domain_suffix": [".b.com"], "domain_keyword": "c", "ip_cidr": ["10.0.0.0/8"], "domain_regex": ["^x$"]}]}"#;

        // Act
        let p = parse(text, Format::Auto);

        // Assert
        let mut got = rules(&p);
        got.sort();
        assert_eq!(
            got,
            [
                (None, RuleKind::Domain, "a.com"),
                (None, RuleKind::Suffix, "b.com"),
                (None, RuleKind::Keyword, "c"),
                (None, RuleKind::Cidr, "10.0.0.0/8")
            ]
        );
        assert_eq!(p.problems.len(), 1);
    }

    #[test]
    fn clash_rule_provider_yaml() {
        // Arrange: a domain set, where a bare domain means that domain only.
        let text = "payload:\n  - '+.youtube.com'\n  - 'youtu.be'\n  - DOMAIN-KEYWORD,youtube\n  - 172.110.32.0/21\n";

        // Act
        let p = parse(text, Format::Auto);

        // Assert
        assert_eq!(p.format, Format::Yaml);
        assert_eq!(
            rules(&p),
            [
                (None, RuleKind::Suffix, "youtube.com"),
                (None, RuleKind::Domain, "youtu.be"),
                (None, RuleKind::Keyword, "youtube"),
                (None, RuleKind::Cidr, "172.110.32.0/21")
            ]
        );
    }

    #[test]
    fn broken_input_is_a_problem_not_a_failure() {
        // Act
        let p = parse("{\"a\": [", Format::Json);

        // Assert
        assert!(p.entries.is_empty());
        assert!(p.problems[0].reason.contains("JSON"));
    }
}
