//! Source-local patches apply before the effective provider settings are decoded.
use crate::catalog::{OVERRIDE_PARAMS_FIELD, OVERRIDE_TEXT_FIELDS};
use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;
use toml::{Table, Value};

#[derive(Clone, Copy)]
enum Map {
    Overrides,
    Tool,
    Parameters,
    Mappings,
}

enum Entry {
    Branch(Map),
    Text,
    Mapping,
}

impl Map {
    fn entry(self, key: &str) -> Option<Entry> {
        Some(match (self, key) {
            (Self::Overrides, _) => Entry::Branch(Self::Tool),
            (Self::Tool, OVERRIDE_PARAMS_FIELD) => Entry::Branch(Self::Parameters),
            (Self::Tool, field) if OVERRIDE_TEXT_FIELDS.contains(&field) => Entry::Text,
            (Self::Parameters, _) => Entry::Text,
            (Self::Mappings, _) => Entry::Mapping,
            _ => return None,
        })
    }
}

pub(super) fn compose(
    inherited: &mut Table,
    local: &mut Table,
    origins: &mut BTreeMap<String, String>,
    declaring: &str,
) -> Result<()> {
    for (name, patch_name, map) in [
        ("overrides", "overrides_patch", Map::Overrides),
        (
            "response_mappings",
            "response_mappings_patch",
            Map::Mappings,
        ),
    ] {
        let Some(patch) = local.remove(patch_name) else {
            continue;
        };
        ensure!(
            !local.contains_key(name),
            "{name} and {patch_name} cannot both be declared in one source"
        );
        let Value::Table(patch) = patch else {
            anyhow::bail!("{patch_name} must be a table");
        };
        if patch.is_empty() {
            continue;
        }

        let original = inherited.remove(name);
        let mut table = to_table(original, name)?;
        let mut leaves = BTreeMap::new();
        let path = format!("/{name}");
        if let Some(origin) = origins.get(name) {
            record_origins(map, &table, &path, origin, &mut leaves);
        }
        if apply(map, &mut table, patch, &path, declaring, &mut leaves)? {
            let mut owners = leaves.values();
            let origin = match owners.next() {
                Some(first) if owners.all(|owner| owner == first) => first.as_str(),
                Some(_) => "mixed",
                None => declaring,
            };
            origins.insert(name.to_owned(), origin.to_owned());
            origins.extend(leaves);
        }
        if !table.is_empty() || origins.contains_key(name) {
            inherited.insert(name.to_owned(), Value::Table(table));
        }
    }
    Ok(())
}

fn to_table(value: Option<Value>, path: &str) -> Result<Table> {
    match value {
        None => Ok(Table::new()),
        Some(Value::Table(table)) => Ok(table),
        Some(_) => anyhow::bail!("{path} must be a table to apply a patch"),
    }
}

fn child_pointer(parent: &str, key: &str) -> String {
    format!("{parent}/{}", key.replace('~', "~0").replace('/', "~1"))
}

fn record_origins(
    map: Map,
    table: &Table,
    path: &str,
    origin: &str,
    leaves: &mut BTreeMap<String, String>,
) {
    for (key, value) in table {
        let path = child_pointer(path, key);
        if let (Some(Entry::Branch(branch)), Value::Table(nested)) = (map.entry(key), value)
            && !nested.is_empty()
        {
            record_origins(branch, nested, &path, origin, leaves);
        } else {
            leaves.insert(path, origin.to_owned());
        }
    }
}

fn drop_origins(leaves: &mut BTreeMap<String, String>, path: &str) {
    leaves.remove(path);
    // '/' immediately precedes '0', so this range contains only descendants.
    leaves
        .extract_if(format!("{path}/")..format!("{path}0"), |_, _| true)
        .for_each(drop);
}

fn apply(
    map: Map,
    inherited: &mut Table,
    patch: Table,
    path: &str,
    source: &str,
    leaves: &mut BTreeMap<String, String>,
) -> Result<bool> {
    let mut changed = false;
    for (key, value) in patch {
        let path = child_pointer(path, &key);
        let entry = map
            .entry(&key)
            .with_context(|| format!("unknown override patch field {path}"))?;
        if value == Value::Boolean(false) {
            if inherited.remove(&key).is_some() {
                drop_origins(leaves, &path);
                changed = true;
            }
            continue;
        }
        match (entry, value) {
            (Entry::Branch(branch), Value::Table(patch)) => {
                if patch.is_empty() {
                    continue;
                }
                let existed = inherited.contains_key(&key);
                let mut nested = to_table(inherited.remove(&key), &path)?;
                let edited = apply(branch, &mut nested, patch, &path, source, leaves)?;
                if edited {
                    leaves.remove(&path);
                    changed = true;
                }
                if !nested.is_empty() || existed && !edited {
                    inherited.insert(key, Value::Table(nested));
                }
            }
            (Entry::Text, value @ Value::String(_)) | (Entry::Mapping, value @ Value::Table(_)) => {
                leaves.insert(path, source.to_owned());
                inherited.insert(key, value);
                changed = true;
            }
            (Entry::Text, _) => anyhow::bail!("{path} must be a string or false"),
            _ => anyhow::bail!("{path} must be a table or false"),
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleting_a_branch_preserves_origins_of_similarly_named_siblings() {
        let parent = "/overrides/read";
        let sibling = [
            "/overrides/read-2/description",
            "/overrides/read~1more/description",
        ];
        let mut leaves = [parent, "/overrides/read/params/a~1b"]
            .into_iter()
            .chain(sibling)
            .map(|key| (key.to_owned(), "provider.toml".to_owned()))
            .collect();

        drop_origins(&mut leaves, parent);

        assert_eq!(
            leaves.keys().map(String::as_str).collect::<Vec<_>>(),
            sibling
        );
    }
}
