//! Apply optional configuration edits without changing the routed contract.
use super::ToolSpec;
use serde::Serialize;
use serde_json::Value;

pub(crate) const OVERRIDE_TEXT_FIELDS: [&str; 2] = ["description", "pricing_description"];
pub(crate) const OVERRIDE_PARAMS_FIELD: &str = "params";

#[derive(Clone, Debug, Serialize)]
pub struct OverrideIssue {
    pub tool: Option<String>,
    pub field: &'static str,
    pub target: Option<String>,
    pub problem: OverrideProblem,
    pub selected: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverrideProblem {
    Malformed,
    UnknownField,
    Missing,
    Ambiguous,
    DuplicateTarget,
    HelpTariff,
}

impl OverrideIssue {
    pub(crate) fn discarded(&self) -> bool {
        self.problem != OverrideProblem::Ambiguous
    }
}

impl std::fmt::Display for OverrideIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(tool) = &self.tool {
            write!(f, "tool {tool} ")?;
        }
        write!(f, "override {}", self.field)?;
        if let Some(target) = &self.target {
            write!(f, " {target:?}")?;
        }
        let reason = match self.problem {
            OverrideProblem::Malformed => "invalid value type",
            OverrideProblem::UnknownField => "unknown field",
            OverrideProblem::Missing => "parameter does not exist",
            OverrideProblem::Ambiguous => "multiple parameter matches",
            OverrideProblem::DuplicateTarget => "another note edits the same parameter",
            OverrideProblem::HelpTariff => "help has no operation tariff",
        };
        write!(f, " ({reason})")?;
        if let Some(selected) = &self.selected {
            write!(f, ", using {selected:?}")?;
        }
        Ok(())
    }
}

impl ToolSpec {
    /// Parameter-note candidates in routing-compatible legacy precedence.
    pub fn parameter_note_pointers(&self, key: &str) -> Vec<String> {
        let escape = |name: &str| name.replace('~', "~0").replace('/', "~1");
        let mut pointers = Vec::new();
        let Some(properties) = self
            .input_schema
            .get("properties")
            .and_then(Value::as_object)
        else {
            return pointers;
        };
        if properties.get(key).is_some_and(Value::is_object) {
            pointers.push(format!("/properties/{}", escape(key)));
        }
        if let Some(body) = &self.whole_body
            && let Some(nested) = properties.get(&body.name).and_then(|p| p.get("properties"))
        {
            let legacy = key
                .strip_suffix("_body")
                .filter(|_| nested.get(key).is_none_or(Value::is_object));
            for wire in [Some(key), legacy].into_iter().flatten() {
                if nested.get(wire).is_some_and(Value::is_object) {
                    pointers.push(format!(
                        "/properties/{}/properties/{}",
                        escape(&body.name),
                        escape(wire)
                    ));
                }
            }
        }
        pointers
    }

    pub fn parameter_note_pointer(&self, key: &str) -> Option<String> {
        self.parameter_note_pointers(key).into_iter().next()
    }
}

pub(super) fn container_issues(overrides: &Value) -> Vec<OverrideIssue> {
    let mut issues = Vec::new();
    let mut report = |tool: Option<&str>, field, target: Option<&str>, problem| {
        issues.push(OverrideIssue {
            tool: tool.map(str::to_owned),
            field,
            target: target.map(str::to_owned),
            problem,
            selected: None,
        });
    };
    let Some(entries) = overrides.as_object() else {
        report(None, "overrides", None, OverrideProblem::Malformed);
        return issues;
    };
    for (name, entry) in entries {
        let tool = Some(name.as_str());
        let Some(patch) = entry.as_object() else {
            report(tool, "overrides", None, OverrideProblem::Malformed);
            continue;
        };
        for (field, value) in patch {
            let text_field = OVERRIDE_TEXT_FIELDS
                .into_iter()
                .find(|name| field.as_str() == *name);
            match text_field {
                Some(_) if value.is_string() => (),
                Some(name) => report(tool, name, None, OverrideProblem::Malformed),
                None if field == OVERRIDE_PARAMS_FIELD => {
                    let Some(notes) = value.as_object() else {
                        report(
                            tool,
                            OVERRIDE_PARAMS_FIELD,
                            None,
                            OverrideProblem::Malformed,
                        );
                        continue;
                    };
                    for (key, _) in notes.iter().filter(|(_, value)| !value.is_string()) {
                        report(
                            tool,
                            OVERRIDE_PARAMS_FIELD,
                            Some(key),
                            OverrideProblem::Malformed,
                        );
                    }
                }
                None => report(
                    tool,
                    "overrides",
                    Some(field),
                    OverrideProblem::UnknownField,
                ),
            }
        }
    }
    issues
}

pub(super) fn check_issues(issues: &[OverrideIssue]) -> anyhow::Result<()> {
    let errors: Vec<_> = issues
        .iter()
        .filter(|issue| issue.discarded())
        .map(ToString::to_string)
        .collect();
    if errors.is_empty() {
        return Ok(());
    }
    Err(super::ToolConfigurationError {
        error: anyhow::anyhow!("{}", errors.join(", ")),
        reason: "Provider tool overrides contain invalid fields, values, or targets",
    }
    .into())
}

pub(super) fn apply(overrides: &Value, tool: &mut ToolSpec, issues: &mut Vec<OverrideIssue>) {
    let Some(patch) = overrides.get(&tool.name).and_then(Value::as_object) else {
        return;
    };
    let name = tool.name.clone();
    let mut report = |field, target, problem, selected| {
        issues.push(OverrideIssue {
            tool: Some(name.clone()),
            field,
            target,
            problem,
            selected,
        });
    };
    let rendering = tool.description_rendering.as_mut().unwrap();
    if let Some(prose) = patch.get("description").and_then(Value::as_str) {
        rendering.prose = prose.to_owned();
    }
    if let Some(note) = patch.get("pricing_description").and_then(Value::as_str) {
        if tool.help_url.is_none() {
            rendering.authored_price = Some(note.to_owned());
        } else {
            report(
                "pricing_description",
                None,
                OverrideProblem::HelpTariff,
                None,
            );
        }
    }
    if let Some(notes) = patch.get("params").and_then(Value::as_object) {
        apply_notes(tool, notes, report);
    }
}

fn apply_notes(
    tool: &mut ToolSpec,
    notes: &serde_json::Map<String, Value>,
    mut report: impl FnMut(&'static str, Option<String>, OverrideProblem, Option<String>),
) {
    for (key, value) in notes {
        if !value.is_string() {
            continue;
        }
        let pointers = tool.parameter_note_pointers(key);
        let Some((pointer, remaining)) = pointers.split_first() else {
            report("params", Some(key.clone()), OverrideProblem::Missing, None);
            continue;
        };
        if !remaining.is_empty() {
            report(
                "params",
                Some(key.clone()),
                OverrideProblem::Ambiguous,
                Some(pointer.clone()),
            );
        }
        if let Some(exact) = key.strip_suffix("_body")
            && notes.get(exact).is_some_and(Value::is_string)
            && tool.parameter_note_pointer(exact).as_ref() == Some(pointer)
        {
            report(
                "params",
                Some(key.clone()),
                OverrideProblem::DuplicateTarget,
                Some(pointer.clone()),
            );
            continue;
        }
        tool.input_schema
            .pointer_mut(pointer)
            .expect("resolved parameter note")["description"] = value.clone();
    }
}

pub(crate) fn report_discarded(source: Option<&str>, prefix: &str, issues: &[OverrideIssue]) {
    for issue in issues.iter().filter(|issue| issue.discarded()) {
        tracing::warn!(
            source,
            prefix,
            tool = issue.tool.as_deref(),
            field = issue.field,
            target = issue.target.as_deref(),
            problem = ?issue.problem,
            "The catalog ignored an override ({issue})"
        );
    }
}
