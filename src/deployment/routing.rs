//! Deployment-only distribution; declarations establish ownership before catalogs.
use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Targets {
    One(String),
    Many(Vec<String>),
}
impl Targets {
    fn names(&self) -> &[String] {
        match self {
            Self::One(name) => std::slice::from_ref(name),
            Self::Many(names) => names,
        }
    }
    fn excluded(&self) -> bool {
        matches!(self, Self::One(name) if name == "exclude")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SendToolsRule {
    pub to: Targets,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
}

/// Origins are independent of wallet origins and never enter automatic identities.
#[derive(Default, Serialize)]
pub struct EffectiveBindings(pub BTreeMap<String, BTreeMap<String, Vec<String>>>);
impl EffectiveBindings {
    /// Project known edges even when unrelated declarations are invalid. Observation
    /// uses this projection, while serving additionally validates declarations.
    pub fn project(config: &MetaConfig) -> Self {
        let mut edges: BTreeMap<_, BTreeMap<String, Vec<String>>> = config
            .servers
            .keys()
            .map(|name| (name.clone(), BTreeMap::new()))
            .collect();
        for (name, listener) in &config.servers {
            for (index, source) in listener.sources.iter().enumerate() {
                if config
                    .sources
                    .get(source)
                    .is_some_and(|s| s.send_tools.is_none())
                {
                    edges
                        .get_mut(name)
                        .unwrap()
                        .entry(source.clone())
                        .or_default()
                        .push(format!("servers.{name}.sources[{index}]"));
                }
            }
        }
        for (source, settings) in &config.sources {
            for (index, rule) in settings.send_tools.iter().flatten().enumerate() {
                if rule.to.excluded() {
                    continue;
                }
                for target in rule.to.names() {
                    if let Some(sources) = edges.get_mut(target) {
                        sources
                            .entry(source.clone())
                            .or_default()
                            .push(format!("sources.{source}.send_tools[{index}].to"));
                    }
                }
            }
        }
        Self(edges)
    }

    pub fn resolve(config: &MetaConfig) -> Result<Self> {
        let routed = config.sources.values().any(|s| s.send_tools.is_some());
        ensure!(
            !routed || !config.servers.contains_key("exclude"),
            "server name exclude is reserved when sources use send_tools"
        );
        for (id, source) in &config.sources {
            if let Some(rules) = &source.send_tools {
                validate_rules(id, rules, config)?;
            }
        }
        for (name, server) in &config.servers {
            let mut seen = BTreeSet::new();
            for source in &server.sources {
                let settings = config
                    .sources
                    .get(source)
                    .with_context(|| format!("server {name}: unknown source {source}"))?;
                ensure!(
                    seen.insert(source),
                    "server {name}: duplicate source {source}"
                );
                ensure!(
                    settings.send_tools.is_none(),
                    "server {name}: routed source {source} cannot also appear in sources"
                );
            }
        }
        Ok(Self::project(config))
    }

    /// Internal listener views only; the authored deployment remains serializable.
    pub fn listeners(&self, config: &MetaConfig) -> BTreeMap<String, ListenerConfig> {
        config
            .servers
            .iter()
            .map(|(name, server)| {
                let mut effective = server.clone();
                // Preserve legacy ordering, appending routed attachments deterministically.
                effective.sources.extend(
                    self.0[name]
                        .keys()
                        .filter(|id| !server.sources.contains(id))
                        .cloned(),
                );
                (name.clone(), effective)
            })
            .collect()
    }
}

fn validate_rules(id: &str, rules: &[SendToolsRule], config: &MetaConfig) -> Result<()> {
    ensure!(!rules.is_empty(), "source {id}: send_tools cannot be empty");
    for (index, rule) in rules.iter().enumerate() {
        ensure!(
            !rule.to.names().is_empty(),
            "source {id}: send_tools[{index}].to cannot be empty"
        );
        let mut seen = BTreeSet::new();
        if !rule.to.excluded() {
            for target in rule.to.names() {
                ensure!(
                    target != "exclude" && config.servers.contains_key(target),
                    "source {id}: send_tools[{index}] unknown destination {target}"
                );
                ensure!(
                    seen.insert(target),
                    "source {id}: send_tools[{index}] duplicate destination {target}"
                );
            }
        }
        match &rule.tools {
            Some(tools) => {
                ensure!(
                    index + 1 != rules.len(),
                    "source {id}: final send_tools rule must be unconditional"
                );
                ensure!(
                    !tools.is_empty(),
                    "source {id}: send_tools[{index}].tools cannot be empty"
                );
                for selector in tools {
                    pattern(selector)?;
                }
            }
            None => ensure!(
                index + 1 == rules.len(),
                "source {id}: unconditional send_tools rule must be last"
            ),
        }
    }
    Ok(())
}

pub(super) struct CompiledRules<'a> {
    rules: &'a [SendToolsRule],
    patterns: Vec<Vec<Regex>>,
    prefix: String,
}
pub(super) struct Decision<'a> {
    pub suffix: &'a str,
    pub rule_index: usize,
    pub selector: Option<&'a str>,
    pub targets: &'a [String],
    pub excluded: bool,
}
impl<'a> CompiledRules<'a> {
    pub fn new(rules: &'a [SendToolsRule], prefix: &str) -> Result<Self> {
        Ok(Self {
            rules,
            prefix: format!("{prefix}_"),
            patterns: rules
                .iter()
                .map(|rule| {
                    rule.tools
                        .iter()
                        .flatten()
                        .map(|selector| pattern(selector))
                        .collect()
                })
                .collect::<Result<_>>()?,
        })
    }
    pub fn decision<'b>(&'b self, name: &'b str) -> Decision<'b> {
        let suffix = name.strip_prefix(&self.prefix).unwrap_or(name);
        for (index, rule) in self.rules.iter().enumerate() {
            let matched = self.patterns[index].iter().position(|p| p.is_match(suffix));
            if rule.tools.is_none() || matched.is_some() {
                return Decision {
                    suffix,
                    rule_index: index,
                    selector: matched.map(|i| rule.tools.as_ref().unwrap()[i].as_str()),
                    targets: if rule.to.excluded() {
                        &[]
                    } else {
                        rule.to.names()
                    },
                    excluded: rule.to.excluded(),
                };
            }
        }
        unreachable!("validated rules have a final catch-all")
    }
    pub fn validate_inventory(&self, id: &str, tools: &[ToolSpec]) -> Result<()> {
        let suffixes: Vec<_> = tools
            .iter()
            .map(|tool| tool.name.strip_prefix(&self.prefix).unwrap_or(&tool.name))
            .collect();
        let mut wins = vec![0; self.rules.len()];
        for tool in tools {
            wins[self.decision(&tool.name).rule_index] += 1;
        }
        for (index, rule) in self.rules.iter().enumerate() {
            for (selector, regex) in rule.tools.iter().flatten().zip(&self.patterns[index]) {
                if !suffixes.iter().any(|suffix| regex.is_match(suffix)) {
                    ensure!(
                        selector.contains(['*', '?']),
                        "source {id}: send_tools[{index}] unknown tool selector {selector}"
                    );
                    tracing::debug!(
                        source = id,
                        rule = index,
                        selector,
                        "Source distribution pattern matches no available tools"
                    );
                }
            }
            if wins[index] == 0 {
                tracing::debug!(
                    source = id,
                    rule = index,
                    "Source distribution rule assigns no current tools; earlier rules may consume its matches"
                );
            }
        }
        Ok(())
    }
}

impl Source {
    pub(super) fn with_send_tools(
        mut self,
        id: &str,
        send_tools: Option<Vec<SendToolsRule>>,
    ) -> Result<Self> {
        if let Some(rules) = &send_tools {
            CompiledRules::new(rules, self.config.prefix.as_deref().unwrap())
                .and_then(|compiled| compiled.validate_inventory(id, &self.tools))
                .context(catalog::diagnostics::Problem(
                    "Source distribution tool selectors are invalid; check the source send_tools rules against its available tools",
                ))
                .context(catalog::LoadStage::Selection)?;
        }
        self.send_tools = send_tools;
        Ok(self)
    }
}

impl Deployment {
    pub fn explain_routing(&self) -> Result<serde_json::Value> {
        let mut explained = BTreeMap::new();
        for (id, source) in &self.sources {
            let compiled = source
                .send_tools
                .as_ref()
                .map(|rules| CompiledRules::new(rules, source.config.prefix.as_deref().unwrap()))
                .transpose()?;
            let prefix = format!("{}_", source.config.prefix.as_deref().unwrap());
            let mut decisions = Vec::new();
            for tool in &source.tools {
                let decision = compiled.as_ref().map(|rules| rules.decision(&tool.name));
                let targets: Vec<_> = match &decision {
                    Some(d) => d.targets.to_vec(),
                    None => self
                        .wallet_resolution
                        .source_bindings
                        .0
                        .iter()
                        .filter(|(_, sources)| sources.contains_key(id))
                        .map(|(name, _)| name.clone())
                        .collect(),
                };
                let effective: Vec<_> = targets
                    .iter()
                    .filter(|name| {
                        self.selected[*name]
                            .iter()
                            .any(|(s, t)| s == id && t.name == tool.name)
                    })
                    .cloned()
                    .collect();
                let wallets: BTreeMap<_, _> = effective
                    .iter()
                    .map(|name| (name.clone(), &self.wallet_resolution.bindings[name][id]))
                    .collect();
                decisions.push(serde_json::json!({"name":tool.name,
                    "suffix":decision.as_ref().map_or_else(|| tool.name.strip_prefix(&prefix).unwrap_or(&tool.name), |d| d.suffix), "method":tool.method,"path":tool.path,
                    "rule_index":decision.as_ref().map(|d| d.rule_index),
                    "selector":decision.as_ref().and_then(|d| d.selector),
                    "catch_all":decision.as_ref().map(|d| d.selector.is_none()),
                    "action":if decision.as_ref().is_some_and(|d|d.excluded) {"exclude"} else {"assign"},
                    "routed_targets":targets,"effective_targets":effective,"wallet_bindings":wallets}));
            }
            explained.insert(id, decisions);
        }
        Ok(serde_json::to_value(explained)?)
    }
}
