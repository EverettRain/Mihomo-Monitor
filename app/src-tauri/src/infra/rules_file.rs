//! The rules file: host groups and attribution rules in one YAML document, to export, edit in
//! any editor and import back whole.
//!
//! ```yaml
//! version: 1
//! groups:
//!   - name: YouTube
//!     note: ""
//!     entries:
//!       - DOMAIN-SUFFIX,youtube.com
//!       - IP-CIDR,91.108.4.0/22
//! attribution:
//!   - name: "Container: us-workstation"
//!     inbound: US-Workstation-Port
//!     priority: 10
//!     enabled: true
//! ```
//!
//! Entries are written as Clash rule lines and read like any typed entry, so `youtube.com` or
//! `+.youtube.com` work too. A section the file leaves out is not part of it: importing such a
//! file leaves that part alone. JSON of the same shape reads as well.

use super::import::Problem;
use crate::domain::attribution::{AttributionRule, Field};
use crate::domain::hostgroup::{parse_entry, Rule, RuleKind};
use yaml_rust2::{Yaml, YamlLoader};

/// The format this app writes. Files of a later version are refused rather than half read.
pub const VERSION: i64 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupSpec {
    pub name: String,
    pub note: String,
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RulesFile {
    /// `None` when the file has no `groups` section.
    pub groups: Option<Vec<GroupSpec>>,
    /// `None` when the file has no `attribution` section.
    pub attribution: Option<Vec<AttributionRule>>,
}

fn line(r: &Rule) -> String {
    let head = match r.kind {
        RuleKind::Domain => "DOMAIN",
        RuleKind::Suffix => "DOMAIN-SUFFIX",
        RuleKind::Keyword => "DOMAIN-KEYWORD",
        RuleKind::Cidr => "IP-CIDR",
    };
    format!("{head},{}", r.value)
}

/// A YAML double-quoted scalar: safe for any text a name or note may hold.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The file for these groups (in order of precedence) and attribution rules.
pub fn render(groups: &[GroupSpec], attribution: &[AttributionRule]) -> String {
    let mut out = String::from(
        "# Mihomo-Monitor 规则配置：域名分组与归属规则。\n\
         # 条目写法与 Clash 规则相同：DOMAIN-SUFFIX、DOMAIN、DOMAIN-KEYWORD、IP-CIDR，也可以只写域名（含子域名）。\n\
         # 分组按先后决定优先级；归属规则只用于内核没有给出进程名的连接，按 inbound（入站名称）精确匹配，priority 小的先匹配。\n\
         # 导入时默认以本文件为准：文件里没有的分组或规则会被删除；删掉整个 groups 或 attribution 段则那部分保持不变。\n",
    );
    out.push_str(&format!("version: {VERSION}\n"));
    out.push_str(if groups.is_empty() { "groups: []\n" } else { "groups:\n" });
    for g in groups {
        out.push_str(&format!("  - name: {}\n    note: {}\n", quoted(&g.name), quoted(&g.note)));
        if g.rules.is_empty() {
            out.push_str("    entries: []\n");
        } else {
            out.push_str("    entries:\n");
            for r in &g.rules {
                out.push_str(&format!("      - {}\n", quoted(&line(r))));
            }
        }
    }
    out.push_str(if attribution.is_empty() { "attribution: []\n" } else { "attribution:\n" });
    for a in attribution {
        let Field::Inbound = a.field;
        out.push_str(&format!(
            "  - name: {}\n    inbound: {}\n    priority: {}\n    enabled: {}\n",
            quoted(&a.name),
            quoted(&a.value),
            a.priority,
            a.enabled
        ));
    }
    out
}

fn text_of(y: &Yaml) -> Option<String> {
    match y {
        Yaml::String(s) => Some(s.clone()),
        Yaml::Integer(i) => Some(i.to_string()),
        Yaml::Real(r) => Some(r.clone()),
        _ => None,
    }
}

fn problem(text: String, reason: impl Into<String>) -> Problem {
    Problem { at: 0, text, reason: reason.into() }
}

/// Reads a rules file. Problems with single groups, entries or rules are reported and those
/// left out; `Err` is for a file that cannot be read as one at all.
pub fn parse(text: &str) -> Result<(RulesFile, Vec<Problem>), String> {
    let docs = YamlLoader::load_from_str(text).map_err(|e| format!("无法读取为 YAML 或 JSON：{e}"))?;
    let Some(doc) = docs.into_iter().next() else { return Err("文件是空的。".into()) };
    let Yaml::Hash(_) = doc else { return Err("这不是规则配置文件：顶层应有 groups 或 attribution。".into()) };
    if let Some(v) = doc["version"].as_i64() {
        if v > VERSION {
            return Err("这个文件来自更新版本的 Mihomo-Monitor，请升级后再导入。".into());
        }
    }
    let mut problems = Vec::new();
    let groups = match &doc["groups"] {
        Yaml::BadValue => None,
        Yaml::Array(items) => Some(read_groups(items, &mut problems)),
        Yaml::Null => Some(Vec::new()),
        _ => return Err("groups 应是一个列表。".into()),
    };
    let attribution = match &doc["attribution"] {
        Yaml::BadValue => None,
        Yaml::Array(items) => Some(read_attribution(items, &mut problems)),
        Yaml::Null => Some(Vec::new()),
        _ => return Err("attribution 应是一个列表。".into()),
    };
    if groups.is_none() && attribution.is_none() {
        return Err("这不是规则配置文件：没有 groups 或 attribution。".into());
    }
    Ok((RulesFile { groups, attribution }, problems))
}

fn read_groups(items: &[Yaml], problems: &mut Vec<Problem>) -> Vec<GroupSpec> {
    let mut out: Vec<GroupSpec> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let at = format!("第 {} 个分组", i + 1);
        let Some(name) = text_of(&item["name"]).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
            problems.push(problem(at, "缺少 name，已跳过"));
            continue;
        };
        if out.iter().any(|g| g.name == name) {
            problems.push(problem(format!("分组「{name}」"), "与前面的分组重名，已跳过"));
            continue;
        }
        let note = text_of(&item["note"]).unwrap_or_default().trim().to_string();
        let mut rules: Vec<Rule> = Vec::new();
        let entries = match &item["entries"] {
            Yaml::Array(list) => list.as_slice(),
            Yaml::BadValue | Yaml::Null => &[],
            _ => {
                problems.push(problem(format!("分组「{name}」"), "entries 应是一个列表，条目已跳过"));
                &[]
            }
        };
        for (j, e) in entries.iter().enumerate() {
            let Some(text) = text_of(e) else {
                problems.push(problem(format!("分组「{name}」第 {} 条", j + 1), "条目应是一行文字"));
                continue;
            };
            match parse_entry(&text, RuleKind::Suffix) {
                Ok(Some(rule)) if !rules.contains(&rule) => rules.push(rule),
                Ok(_) => {}
                Err(reason) => problems.push(problem(format!("分组「{name}」第 {} 条：{text}", j + 1), reason)),
            }
        }
        out.push(GroupSpec { name, note, rules });
    }
    out
}

fn read_attribution(items: &[Yaml], problems: &mut Vec<Problem>) -> Vec<AttributionRule> {
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let at = format!("第 {} 条归属规则", i + 1);
        let priority = match &item["priority"] {
            Yaml::BadValue | Yaml::Null => 100,
            p => match p.as_i64() {
                Some(p) => p,
                None => {
                    problems.push(problem(at, "priority 应是整数，已跳过"));
                    continue;
                }
            },
        };
        let enabled = match &item["enabled"] {
            Yaml::BadValue | Yaml::Null => true,
            e => match e.as_bool() {
                Some(e) => e,
                None => {
                    problems.push(problem(at, "enabled 应是 true 或 false，已跳过"));
                    continue;
                }
            },
        };
        let rule = AttributionRule {
            id: None,
            name: text_of(&item["name"]).unwrap_or_default(),
            field: Field::Inbound,
            value: text_of(&item["inbound"]).unwrap_or_default(),
            priority,
            enabled,
        };
        match rule.checked() {
            Ok(rule) => out.push(rule),
            Err(reason) => problems.push(problem(at, reason)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(kind: RuleKind, value: &str) -> Rule {
        Rule::new(kind, value).unwrap()
    }

    #[test]
    fn what_is_written_reads_back_the_same() {
        // Arrange: names that need quoting, every kind of entry, and both kinds of rule state.
        let groups = vec![
            GroupSpec {
                name: "YouTube".into(),
                note: "视频：含 \"引号\" 与 # 号".into(),
                rules: vec![
                    rule(RuleKind::Suffix, "youtube.com"),
                    rule(RuleKind::Domain, "www.google.com"),
                    rule(RuleKind::Keyword, "ytimg"),
                    rule(RuleKind::Cidr, "91.108.4.0/22"),
                ],
            },
            GroupSpec { name: "空分组".into(), note: String::new(), rules: vec![] },
        ];
        let attribution = vec![
            AttributionRule {
                id: Some(3),
                name: "Container: us-workstation".into(),
                field: Field::Inbound,
                value: "US-Workstation-Port".into(),
                priority: 10,
                enabled: true,
            },
            AttributionRule {
                id: Some(4),
                name: "Homelab NAS".into(),
                field: Field::Inbound,
                value: "LAN-NAS-Port".into(),
                priority: 20,
                enabled: false,
            },
        ];

        // Act
        let (file, problems) = parse(&render(&groups, &attribution)).unwrap();

        // Assert
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(file.groups, Some(groups));
        assert_eq!(
            file.attribution.unwrap().iter().map(|a| AttributionRule { id: None, ..a.clone() }).collect::<Vec<_>>(),
            attribution.into_iter().map(|a| AttributionRule { id: None, ..a }).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_hand_edited_file_reads_with_its_mistakes_listed() {
        // Arrange: loose entries, a duplicate group, a group without a name, a bad rule.
        let text = "groups:\n  - name: 开发\n    entries: [github.com, '+.githubusercontent.com', 'GEOIP,CN', 10.0.0.0/8]\n  - name: 开发\n  - note: 没有名字\nattribution:\n  - name: NAS\n    inbound: LAN-NAS-Port\n  - name: ''\n    inbound: X\n";

        // Act
        let (file, problems) = parse(text).unwrap();

        // Assert
        let groups = file.groups.unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].rules,
            vec![rule(RuleKind::Suffix, "github.com"), rule(RuleKind::Suffix, "githubusercontent.com"), rule(RuleKind::Cidr, "10.0.0.0/8")]
        );
        let nas = &file.attribution.unwrap()[0];
        assert_eq!((nas.priority, nas.enabled), (100, true));
        assert_eq!(problems.len(), 4, "{problems:?}");
    }

    #[test]
    fn a_section_left_out_is_not_part_of_the_file() {
        // Act
        let (only_rules, _) = parse("{\"attribution\": []}").unwrap();
        let newer = parse("version: 2\ngroups: []");
        let unrelated = parse("payload:\n  - youtube.com\n");

        // Assert
        assert_eq!(only_rules, RulesFile { groups: None, attribution: Some(vec![]) });
        assert!(newer.is_err());
        assert!(unrelated.is_err());
    }
}
