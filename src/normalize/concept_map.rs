//! The canonical statement template: screener items mapped onto XBRL tags.
//!
//! Loaded from `config/concepts.toml` (embedded at compile time). Tags are
//! listed in priority order — when several tags carry a value for the same
//! company-period, the earliest listed wins. Priorities encode reporting
//! history (ASC 606 revenue tags outrank the pre-606 ones), so order is
//! load-bearing.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::Error;
use crate::Result;

/// The default template, compiled into the binary so the CLI has no runtime
/// config path to resolve.
pub const DEFAULT_TOML: &str = include_str!("../../config/concepts.toml");

/// `us-gaap:CostsAndExpenses` is a total that includes SG&A. Mapping it to
/// any cost item corrupts gross margins while making coverage look better —
/// rejected at load, not by convention.
const FORBIDDEN_TAGS: [&str; 1] = ["CostsAndExpenses"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub key: String,
    pub label: String,
    /// `income`, `balance`, or `cashflow`.
    pub statement: String,
    /// `flow` (duration) or `instant` (balance-sheet level).
    pub kind: String,
    /// XBRL unit the tags are meaningful in (`USD`, `shares`, `USD/shares`).
    pub unit: String,
    /// Priority-ordered us-gaap tags; index is rank, lower wins.
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ConceptMap {
    pub items: Vec<Item>,
}

#[derive(Deserialize)]
struct FileSpec {
    items: BTreeMap<String, ItemSpec>,
}

#[derive(Deserialize)]
struct ItemSpec {
    label: String,
    statement: String,
    kind: String,
    unit: String,
    tags: Vec<String>,
}

fn invalid(msg: String) -> Error {
    Error::ConceptMap(msg)
}

pub fn parse(raw: &str) -> Result<ConceptMap> {
    let spec: FileSpec = toml::from_str(raw).map_err(|e| invalid(format!("concepts.toml: {e}")))?;

    let mut items = Vec::with_capacity(spec.items.len());
    for (key, item) in spec.items {
        if !["income", "balance", "cashflow"].contains(&item.statement.as_str()) {
            return Err(invalid(format!(
                "{key}: unknown statement {:?}",
                item.statement
            )));
        }
        if !["flow", "instant"].contains(&item.kind.as_str()) {
            return Err(invalid(format!("{key}: unknown kind {:?}", item.kind)));
        }
        if item.tags.is_empty() {
            return Err(invalid(format!("{key}: no tags")));
        }
        for tag in &item.tags {
            if FORBIDDEN_TAGS.contains(&tag.as_str()) {
                return Err(invalid(format!(
                    "{key}: {tag} is forbidden — it is an aggregate that would \
                     corrupt the item it pretends to fill"
                )));
            }
        }
        items.push(Item {
            key,
            label: item.label,
            statement: item.statement,
            kind: item.kind,
            unit: item.unit,
            tags: item.tags,
        });
    }
    Ok(ConceptMap { items })
}

pub fn default() -> Result<ConceptMap> {
    parse(DEFAULT_TOML)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_loads_and_validates() {
        let map = default().unwrap();
        assert!(map.items.len() >= 15);
        let revenue = map.items.iter().find(|i| i.key == "revenue").unwrap();
        // ASC 606 tag outranks the legacy Revenues tag.
        assert!(
            revenue
                .tags
                .iter()
                .position(|t| t == "RevenueFromContractWithCustomerExcludingAssessedTax")
                < revenue.tags.iter().position(|t| t == "Revenues")
        );
    }

    #[test]
    fn costs_and_expenses_is_rejected_anywhere() {
        let raw = r#"
            [items.cost_of_revenue]
            label = "COGS"
            statement = "income"
            kind = "flow"
            unit = "USD"
            tags = ["CostsAndExpenses"]
        "#;
        let err = parse(raw).unwrap_err().to_string();
        assert!(err.contains("forbidden"), "{err}");
    }

    #[test]
    fn unknown_statement_or_kind_is_rejected() {
        let raw = r#"
            [items.x]
            label = "X"
            statement = "vibes"
            kind = "flow"
            unit = "USD"
            tags = ["Assets"]
        "#;
        assert!(parse(raw).is_err());
    }
}
