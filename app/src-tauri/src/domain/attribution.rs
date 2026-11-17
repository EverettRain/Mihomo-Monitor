//! Naming traffic the core could not tie to a process.
//!
//! mihomo names a connection's process only when it can look it up (TUN mode, or
//! `find-process-mode`); connections from containers or other machines, which reach the core
//! through a port of their own, come without one. A rule names such traffic by a fact the core
//! does record for every connection, the inbound it came in through: a connection without a
//! process that matches a rule is counted under the rule's name. A process name the core found is
//! never replaced, and nothing is guessed from facts many programs share, such as 127.0.0.1 or a
//! service's domain.

use super::split::Conn;

/// What a rule looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Field {
    /// The name of the inbound (`metadata.inboundName`), compared exactly.
    Inbound,
}

impl Field {
    pub fn as_str(self) -> &'static str {
        match self {
            Field::Inbound => "inbound",
        }
    }

    pub fn parse(s: &str) -> Option<Field> {
        match s {
            "inbound" => Some(Field::Inbound),
            _ => None,
        }
    }
}

/// The longest name a rule may give, in characters.
pub const NAME_MAX: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributionRule {
    /// `None` for a rule not saved yet.
    pub id: Option<i64>,
    /// Shown as the process of the traffic the rule matches.
    pub name: String,
    pub field: Field,
    pub value: String,
    /// Rules are tried from the lowest number up.
    pub priority: i64,
    pub enabled: bool,
}

impl AttributionRule {
    /// The rule as it should be saved, or why it cannot be.
    pub fn checked(mut self) -> Result<AttributionRule, String> {
        self.name = self.name.trim().to_string();
        self.value = self.value.trim().to_string();
        if self.name.is_empty() {
            return Err("请填写显示名称。".into());
        }
        if self.name.chars().count() > NAME_MAX {
            return Err(format!("显示名称不能超过 {NAME_MAX} 个字。"));
        }
        if self.value.is_empty() {
            return Err("请填写入站名称。".into());
        }
        Ok(self)
    }

    fn matches(&self, c: &Conn) -> bool {
        match self.field {
            Field::Inbound => c.inbound == self.value,
        }
    }
}

/// The enabled rules, in the order they are tried.
#[derive(Debug, Default)]
pub struct Attributor {
    rules: Vec<AttributionRule>,
}

impl Attributor {
    /// Lower priority numbers first; between equal numbers, the rule saved first.
    pub fn new(rules: impl IntoIterator<Item = AttributionRule>) -> Attributor {
        let mut rules: Vec<AttributionRule> = rules.into_iter().filter(|r| r.enabled).collect();
        rules.sort_by_key(|r| (r.priority, r.id));
        Attributor { rules }
    }

    /// The process a connection's traffic is counted under: the core's own name when it has one,
    /// else the name of the first rule that matches, else empty (not identified).
    pub fn process_of<'a>(&'a self, c: &'a Conn) -> &'a str {
        if !c.process.is_empty() {
            return &c.process;
        }
        self.rules.iter().find(|r| r.matches(c)).map_or("", |r| r.name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::counter::Bytes;

    fn conn(process: &str, inbound: &str) -> Conn {
        Conn {
            id: "1".into(),
            bytes: Bytes::ZERO,
            chains: vec!["DIRECT".into()],
            provider_chains: vec![String::new()],
            process: process.into(),
            inbound: inbound.into(),
            host: "example.com".into(),
        }
    }

    fn rule(id: i64, name: &str, inbound: &str, priority: i64) -> AttributionRule {
        AttributionRule { id: Some(id), name: name.into(), field: Field::Inbound, value: inbound.into(), priority, enabled: true }
    }

    #[test]
    fn the_lowest_priority_number_wins() {
        // Arrange: saved in the opposite order to their priority.
        let rules =
            Attributor::new([rule(1, "后备", "US-Workstation-Port", 20), rule(2, "Container: us-workstation", "US-Workstation-Port", 10)]);

        // Act
        let c = conn("", "US-Workstation-Port");
        let name = rules.process_of(&c);

        // Assert
        assert_eq!(name, "Container: us-workstation");
    }

    #[test]
    fn equal_priorities_go_by_the_rule_saved_first() {
        // Arrange
        let rules = Attributor::new([rule(2, "后建", "Port-A", 5), rule(1, "先建", "Port-A", 5)]);

        // Act
        let c = conn("", "Port-A");
        let name = rules.process_of(&c);

        // Assert
        assert_eq!(name, "先建");
    }

    #[test]
    fn traffic_no_rule_matches_stays_unidentified() {
        // Arrange: matching is exact, and a disabled rule takes no part.
        let mut off = rule(2, "已停用", "DEFAULT-MIXED", 1);
        off.enabled = false;
        let rules = Attributor::new([rule(1, "Container: us-workstation", "US-Workstation-Port", 10), off]);

        // Act
        let names =
            ["DEFAULT-MIXED", "us-workstation-port", "US-Workstation-Port-2", ""].map(|i| rules.process_of(&conn("", i)).to_string());

        // Assert
        assert_eq!(names, ["", "", "", ""].map(String::from));
    }

    #[test]
    fn a_process_the_core_named_is_kept() {
        // Arrange: a rule for the very inbound the connection came in through.
        let rules = Attributor::new([rule(1, "Container: us-workstation", "US-Workstation-Port", 1)]);

        // Act
        let c = conn("curl", "US-Workstation-Port");
        let name = rules.process_of(&c);

        // Assert
        assert_eq!(name, "curl");
    }

    #[test]
    fn a_rule_needs_a_name_and_a_value() {
        // Arrange
        let blank_name = AttributionRule { name: "  ".into(), ..rule(1, "", "Port", 1) };
        let blank_value = AttributionRule { value: " ".into(), ..rule(1, "名称", "", 1) };
        let long_name = AttributionRule { name: "名".repeat(NAME_MAX + 1), ..rule(1, "", "Port", 1) };
        let padded = rule(1, "  容器  ", " Port ", 1);

        // Act
        let results = [blank_name, blank_value, long_name].map(|r| r.checked().is_err());
        let trimmed = padded.checked().unwrap();

        // Assert
        assert_eq!(results, [true, true, true]);
        assert_eq!((trimmed.name.as_str(), trimmed.value.as_str()), ("容器", "Port"));
    }
}
