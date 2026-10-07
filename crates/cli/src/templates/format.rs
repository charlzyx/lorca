//! The portable allowlist. The file contains reusable text, never a serialized runtime bot.

use std::collections::{BTreeMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const FORMAT: &str = "lorca.bot-template";
pub const VERSION: u32 = 1;
pub const MAX_BYTES: usize = 1024 * 1024;
pub const MAX_ITEMS: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub format: String,
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<Skill>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memories: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routines: Vec<Routine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requirements: Vec<Requirement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    pub description: String,
    pub symbol_name: String,
    pub accent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub instructions: String,
    #[serde(default)]
    pub examples: String,
    #[serde(default)]
    pub references: Vec<Resource>,
    #[serde(default)]
    pub scripts: Vec<Resource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Routine {
    pub name: String,
    pub schedule: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missed_run_policy: Option<String>,
}

/// A service, not the exporting user's connection, settings, or sign-in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub service_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Warning {
    pub path: String,
    pub message: String,
}

impl Default for Template {
    fn default() -> Self {
        Self {
            format: FORMAT.into(),
            version: VERSION,
            profile: None,
            skills: vec![],
            memories: vec![],
            routines: vec![],
            requirements: vec![],
        }
    }
}

impl Template {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_BYTES {
            return Err("A template file is at most 1 MiB.".into());
        }
        // Explain the version before decoding fields whose meaning may have changed.
        let value: Value = serde_json::from_str(text)
            .map_err(|e| format!("The template is not valid JSON: {e}"))?;
        if value["format"].as_str() != Some(FORMAT) {
            return Err(format!("This file is not a {FORMAT} file."));
        }
        if value["version"].as_u64() != Some(VERSION as u64) {
            return Err(format!(
                "Unsupported template version {}; this Lorca reads version {VERSION}.",
                value["version"]
            ));
        }
        let template: Self = serde_json::from_value(value)
            .map_err(|e| format!("Unsupported or invalid template field: {e}"))?;
        template.validate()?;
        Ok(template)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT || self.version != VERSION {
            return Err(format!("This Lorca reads {FORMAT} version {VERSION}."));
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_BYTES {
            return Err("A template file is at most 1 MiB.".into());
        }
        if self.skills.len() > MAX_ITEMS
            || self.memories.len() > MAX_ITEMS
            || self.requirements.len() > MAX_ITEMS
        {
            return Err(format!(
                "Select at most {MAX_ITEMS} skills, memories, and requirements each."
            ));
        }
        if self.routines.len() > crate::routines::MAX_PER_BOT {
            return Err(format!(
                "A bot keeps at most {} routines.",
                crate::routines::MAX_PER_BOT
            ));
        }
        if let Some(profile) = &self.profile {
            text_field("profile.name", &profile.name, 200, true)?;
            text_field(
                "profile.description",
                &profile.description,
                256 * 1024,
                false,
            )?;
            text_field("profile.symbol_name", &profile.symbol_name, 100, true)?;
            if ![
                "indigo", "blue", "teal", "green", "orange", "red", "pink", "purple",
            ]
            .contains(&profile.accent.as_str())
            {
                return Err(format!("Unsupported profile.accent {:?}.", profile.accent));
            }
        }
        let mut names = HashSet::new();
        for skill in &self.skills {
            if !is_skill_name(&skill.name) {
                return Err(format!("Invalid skill name {:?}; use a lowercase name with letters, digits, or dashes.", skill.name));
            }
            if !names.insert(&skill.name) {
                return Err(format!("Skill {:?} is listed twice.", skill.name));
            }
            text_field("skill.description", &skill.description, 512, true)?;
            text_field("skill.instructions", &skill.instructions, 64 * 1024, true)?;
            if skill.references.len() > 16 || skill.scripts.len() > 16 {
                return Err("A skill has at most 16 references and 16 scripts.".into());
            }
            text_field("skill.examples", &skill.examples, 64 * 1024, false)?;
            let mut paths = HashSet::new();
            for (prefix, resources) in [
                ("references/", &skill.references),
                ("scripts/", &skill.scripts),
            ] {
                for resource in resources {
                    let path = &resource.path;
                    if !path.starts_with(prefix)
                        || path.len() > 160
                        || path.split('/').any(|part| {
                            part.is_empty()
                                || part == "."
                                || part == ".."
                                || !part.bytes().all(|b| {
                                    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')
                                })
                        })
                    {
                        return Err(format!("Unsafe skill resource path {path:?}; use a relative path under {prefix}."));
                    }
                    if !paths.insert(path) {
                        return Err(format!("Skill resource {path:?} is listed twice."));
                    }
                    text_field("skill.resource.text", &resource.text, 64 * 1024, false)?;
                }
            }
            if serde_json::to_vec(skill).map_err(|e| e.to_string())?.len() > 64 * 1024 {
                return Err("A skill's instructions and resources must fit in 64 KiB.".into());
            }
        }
        let mut memory_bytes = 0;
        for memory in &self.memories {
            text_field("memory", memory, 64 * 1024, true)?;
            memory_bytes += memory.len() + 1;
        }
        if memory_bytes > crate::memory::MEMORY_FILE_MAX_BYTES {
            return Err("The selected memories exceed the memory file's 256 KiB limit.".into());
        }
        let mut names = HashSet::new();
        for routine in &self.routines {
            text_field("routine.name", &routine.name, 240, true)?;
            if routine.name.trim().chars().count() > crate::routines::MAX_NAME_CHARS {
                return Err(format!(
                    "A routine name is at most {} characters.",
                    crate::routines::MAX_NAME_CHARS
                ));
            }
            if !names.insert(
                routine
                    .name
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase(),
            ) {
                return Err(format!("Routine {:?} is listed twice.", routine.name));
            }
            text_field("routine.prompt", &routine.prompt, 64 * 1024, true)?;
            crate::schedule::parse(&routine.schedule)
                .map_err(|e| format!("Routine {:?}: {e}", routine.name))?;
            if let Some(timezone) = &routine.timezone {
                text_field("routine.timezone", timezone, 100, true)?;
            }
            if let Some(policy) = &routine.missed_run_policy {
                if !["coalesce", "skip"].contains(&policy.as_str()) {
                    return Err(format!("Unsupported routine missed_run_policy {policy:?}."));
                }
            }
            if let Some(check) = &routine.check {
                text_field("routine.check", check, 32 * 1024, false)?;
                if check.chars().count() > crate::routines::MAX_CHECK_CHARS {
                    return Err(format!(
                        "A routine check is at most {} characters.",
                        crate::routines::MAX_CHECK_CHARS
                    ));
                }
            }
        }
        let mut ids = HashSet::new();
        for requirement in &self.requirements {
            if !crate::plugins::is_id(&requirement.service_id) || requirement.service_id.len() > 100
            {
                return Err(format!(
                    "Invalid requirement service_id {:?}.",
                    requirement.service_id
                ));
            }
            if !ids.insert(&requirement.service_id) {
                return Err(format!(
                    "Requirement {:?} is listed twice.",
                    requirement.service_id
                ));
            }
        }
        Ok(())
    }

    /// Export scrubs every text field before previewing it. Import refuses a file containing a
    /// recognizable credential, so setup never silently differs from the reviewed file.
    pub fn scrub(&mut self) -> Vec<Warning> {
        let mut warnings = vec![];
        self.each_text_mut(|path, text| {
            let clean = scrub_text(text);
            if clean != *text {
                warnings.push(Warning {
                    path: path.into(),
                    message: "Credential-like text is redacted. Review the remaining content."
                        .into(),
                });
                *text = clean;
            }
        });
        warnings
    }

    pub fn scrub_known(&mut self, secrets: &[String]) -> Vec<Warning> {
        let mut warnings = vec![];
        self.each_text_mut(|path, text| {
            let clean = super::secrets::redact(text, secrets);
            if clean != *text {
                warnings.push(Warning {
                    path: path.into(),
                    message: "A saved credential value is redacted. Review the remaining content."
                        .into(),
                });
                *text = clean;
            }
        });
        warnings
    }

    pub fn warnings(&self) -> Vec<Warning> {
        let mut copy = self.clone();
        let mut warnings = vec![];
        copy.each_text_mut(|path, text| {
            let mut reasons = vec![];
            if path.starts_with("memories.") {
                reasons.push("Selected memory may contain personal or project information");
            }
            if EMAIL.is_match(text) {
                reasons.push("Contains an email address");
            }
            if PATH.is_match(text) {
                reasons.push("Contains a machine path");
            }
            if URL.is_match(text) {
                reasons.push("Contains a URL; review private links and identifiers");
            }
            if PHONE.is_match(text) {
                reasons.push("May contain a phone number");
            }
            if REDACTION.is_match(text) {
                reasons.push("Contains redacted credential text");
            }
            if path.ends_with(".check") || path.contains(".scripts.") {
                reasons.push("Contains a script; review its behavior before running");
            }
            if !reasons.is_empty() {
                warnings.push(Warning {
                    path: path.into(),
                    message: reasons.join(". ") + ".",
                });
            }
        });
        warnings
    }

    /// Normalizes source account namespaces to service namespaces on export, then maps those
    /// namespaces to recipient connections on import. Mapping is simultaneous, so one target
    /// cannot be rewritten as another source. Only explicit tool namespaces are replaced.
    pub fn map_namespaces(&mut self, mappings: &BTreeMap<String, String>) {
        self.each_text_mut(|_, text| {
            *text = NAMESPACE
                .replace_all(text, |caps: &regex::Captures| {
                    let namespace = &caps[1];
                    mappings
                        .get(namespace)
                        .map(|target| {
                            format!("{target}__{}", &caps[2])
                                .chars()
                                .take(64)
                                .collect::<String>()
                        })
                        .unwrap_or_else(|| caps[0].to_string())
                })
                .into_owned();
        });
    }

    pub fn namespaces(&self) -> HashSet<String> {
        let mut copy = self.clone();
        let mut names = HashSet::new();
        copy.each_text_mut(|_, text| {
            for caps in NAMESPACE.captures_iter(text) {
                names.insert(caps[1].to_string());
            }
        });
        names
    }

    pub fn contains_text(&self, needle: &str) -> bool {
        let mut copy = self.clone();
        let mut found = false;
        copy.each_text_mut(|_, text| {
            found |= text.contains(needle);
        });
        found
    }

    pub fn connection_references(&self) -> HashSet<String> {
        let mut copy = self.clone();
        let mut names = HashSet::new();
        copy.each_text_mut(|_, text| {
            for caps in CONNECTION.captures_iter(text) {
                names.insert(caps[1].to_string());
            }
        });
        names
    }

    pub fn resolve_connections(&mut self, mappings: &BTreeMap<String, String>) {
        self.each_text_mut(|_, text| {
            *text = CONNECTION
                .replace_all(text, |caps: &regex::Captures| {
                    mappings
                        .get(&caps[1])
                        .cloned()
                        .unwrap_or_else(|| caps[0].to_string())
                })
                .into_owned();
        });
    }

    pub fn map_connection_ids(&mut self, mappings: &BTreeMap<String, String>) {
        if mappings.is_empty() {
            return;
        }
        let pattern = mappings
            .keys()
            .map(|id| regex::escape(id))
            .collect::<Vec<_>>()
            .join("|");
        let pattern = Regex::new(&format!(r"\b(?:{pattern})\b")).expect("escaped connection ids");
        self.each_text_mut(|_, text| {
            *text = pattern
                .replace_all(text, |caps: &regex::Captures| mappings[&caps[0]].clone())
                .into_owned();
        });
    }

    fn each_text_mut(&mut self, mut apply: impl FnMut(&str, &mut String)) {
        if let Some(profile) = &mut self.profile {
            apply("profile.name", &mut profile.name);
            apply("profile.description", &mut profile.description);
            apply("profile.symbol_name", &mut profile.symbol_name);
        }
        for (i, skill) in self.skills.iter_mut().enumerate() {
            apply(&format!("skills.{i}.name"), &mut skill.name);
            apply(&format!("skills.{i}.description"), &mut skill.description);
            apply(&format!("skills.{i}.instructions"), &mut skill.instructions);
            apply(&format!("skills.{i}.examples"), &mut skill.examples);
            for (kind, resources) in [
                ("references", &mut skill.references),
                ("scripts", &mut skill.scripts),
            ] {
                for (j, resource) in resources.iter_mut().enumerate() {
                    apply(&format!("skills.{i}.{kind}.{j}.path"), &mut resource.path);
                    apply(&format!("skills.{i}.{kind}.{j}.text"), &mut resource.text);
                }
            }
        }
        for (i, memory) in self.memories.iter_mut().enumerate() {
            apply(&format!("memories.{i}"), memory);
        }
        for (i, routine) in self.routines.iter_mut().enumerate() {
            apply(&format!("routines.{i}.name"), &mut routine.name);
            apply(&format!("routines.{i}.prompt"), &mut routine.prompt);
            if let Some(check) = &mut routine.check {
                apply(&format!("routines.{i}.check"), check);
            }
        }
    }
}

pub fn namespace(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn is_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && crate::plugins::is_id(name)
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
}

fn text_field(path: &str, value: &str, max: usize, required: bool) -> Result<(), String> {
    if required && value.trim().is_empty() {
        return Err(format!("{path} is empty."));
    }
    if value.len() > max {
        return Err(format!("{path} is over its {max} byte limit."));
    }
    if value.contains('\0') {
        return Err(format!("{path} contains a NUL character."));
    }
    Ok(())
}

static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b").unwrap());
static PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:/Users/|/home/|[A-Za-z]:\\|~/)[^\s]+").unwrap());
static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://[^\s]+").unwrap());
static PHONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\+\d[\d ()-]{7,}\d").unwrap());
static NAMESPACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b([A-Za-z][A-Za-z0-9_]*?)__([A-Za-z0-9_]*)").unwrap());
static REDACTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"«redacted (?:\d+ chars|credential)»").unwrap());
static CONNECTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{connection:([a-z0-9-]+)\}\}").unwrap());

/// The memory scrubber sees an assignment's redaction marker as another value. Preserve its
/// complete markers so an exported, scrubbed file remains importable without repeated edits.
fn scrub_text(text: &str) -> String {
    let mut out = String::new();
    let mut start = 0;
    for marker in REDACTION.find_iter(text) {
        out.push_str(&crate::memory::scrub(&text[start..marker.start()]));
        out.push_str(marker.as_str());
        start = marker.end();
    }
    out.push_str(&crate::memory::scrub(&text[start..]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn value() -> Value {
        serde_json::to_value(Template::default()).unwrap()
    }

    #[test]
    fn refuses_unknown_fields_versions_and_runtime_data() {
        for field in [
            "credentials",
            "provider",
            "runner_id",
            "messages",
            "workdir",
            "connection_id",
        ] {
            let mut v = value();
            v[field] = json!("sensitive");
            assert!(Template::parse(&v.to_string()).unwrap_err().contains(field));
        }
        let mut v = value();
        v["version"] = json!(2);
        assert!(Template::parse(&v.to_string())
            .unwrap_err()
            .contains("version 2"));
        v = value();
        v["requirements"] = json!([{ "service_id": "github", "token": "secret" }]);
        assert!(Template::parse(&v.to_string())
            .unwrap_err()
            .contains("token"));
    }

    #[test]
    fn bounds_and_validates_embedded_content() {
        let mut v = value();
        v["skills"] = json!([{ "name": "../../credentials", "instructions": "read" }]);
        assert!(Template::parse(&v.to_string()).is_err());
        v["skills"] = json!([]);
        v["routines"] = json!([{ "name": "Check", "schedule": "every 1s", "prompt": "look" }]);
        assert!(Template::parse(&v.to_string()).is_err());
        v["routines"] = json!([{ "name": "Check", "schedule": "every 2h", "prompt": "look", "is_enabled": true }]);
        assert!(Template::parse(&v.to_string())
            .unwrap_err()
            .contains("is_enabled"));
        assert!(Template::parse(&" ".repeat(MAX_BYTES + 1))
            .unwrap_err()
            .contains("1 MiB"));
    }

    #[test]
    fn scrubs_credentials_and_identifies_personal_content() {
        let mut t = Template {
            memories: vec![
                "Email alice@example.com, file /Users/alice/private. API_KEY=abcdef1234567890"
                    .into(),
            ],
            ..Template::default()
        };
        assert_eq!(t.scrub().len(), 1);
        assert!(!t.memories[0].contains("abcdef1234567890"));
        assert!(
            t.scrub().is_empty(),
            "a reviewed redaction is stable on import"
        );
        let warning = &t.warnings()[0];
        assert_eq!(warning.path, "memories.0");
        assert!(
            warning.message.contains("email")
                && warning.message.contains("path")
                && warning.message.contains("personal")
        );
        t.memories.push("API_KEY=«redacted credential»".into());
        assert!(
            t.scrub().is_empty(),
            "playbook redaction markers are stable too"
        );
    }

    #[test]
    fn maps_only_namespaces_simultaneously() {
        let mut t = Template {
            memories: vec![
                "tools.gmail_work__search(); github__read(); gmail_work is a label".into(),
            ],
            ..Template::default()
        };
        t.map_namespaces(&BTreeMap::from([
            ("gmail_work".into(), "github".into()),
            ("github".into(), "recipient".into()),
        ]));
        assert_eq!(
            t.memories[0],
            "tools.github__search(); recipient__read(); gmail_work is a label"
        );
    }

    #[test]
    fn portable_account_references_map_to_services_and_tool_names_fit_the_runtime() {
        let account = "gmail-0123456789abcdef0123456789abcdef";
        let mut template = Template {
            memories: vec![format!(
                "Use {account} with tools.{}__get_thread()",
                namespace(account)
            )],
            ..Template::default()
        };
        template.map_namespaces(&BTreeMap::from([(namespace(account), "gmail".into())]));
        template.map_connection_ids(&BTreeMap::from([(
            account.into(),
            "{{connection:gmail}}".into(),
        )]));
        assert_eq!(
            template.memories[0],
            "Use {{connection:gmail}} with tools.gmail__get_thread()"
        );
        template.resolve_connections(&BTreeMap::from([(
            "gmail".into(),
            "recipient-account".into(),
        )]));
        assert!(template.memories[0].contains("recipient-account"));
        template.memories = vec![format!("tools.gmail__{}()", "a".repeat(50))];
        template.map_namespaces(&BTreeMap::from([("gmail".into(), namespace(account))]));
        assert_eq!(
            template.memories[0]
                .trim_start_matches("tools.")
                .trim_end_matches("()")
                .len(),
            64
        );
    }

    #[test]
    fn validates_playbook_resources_and_normalized_routine_names_before_setup() {
        let mut value = value();
        value["skills"] = json!([{ "name": "review", "description": "Review", "instructions": "Read", "examples": "Read first", "references": [{ "path": "references/../credentials.json", "text": "sensitive" }] }]);
        assert!(Template::parse(&value.to_string())
            .unwrap_err()
            .contains("Unsafe"));
        value["skills"][0]["references"][0]["path"] = json!("references/checklist.md");
        assert!(Template::parse(&value.to_string()).is_ok());
        value["routines"] = json!([{ "name": "Morning   Brief", "schedule": "every 2h", "prompt": "Read" }, { "name": "morning brief", "schedule": "every 2h", "prompt": "Read" }]);
        assert!(Template::parse(&value.to_string())
            .unwrap_err()
            .contains("twice"));
    }

    #[test]
    fn reads_additive_routine_configuration_and_refuses_runtime_fields() {
        let mut v = value();
        v["routines"] = json!([{ "name": "Morning", "schedule": "every 2h", "prompt": "Read", "timezone": "Asia/Singapore", "missed_run_policy": "coalesce" }]);
        let parsed = Template::parse(&v.to_string()).unwrap();
        assert_eq!(
            parsed.routines[0].timezone.as_deref(),
            Some("Asia/Singapore")
        );
        assert_eq!(
            parsed.routines[0].missed_run_policy.as_deref(),
            Some("coalesce")
        );
        for field in ["health", "last_scheduled_at", "last_run_at", "last_outcome"] {
            let mut invalid = v.clone();
            invalid["routines"][0][field] = json!("runner-state");
            assert!(Template::parse(&invalid.to_string())
                .unwrap_err()
                .contains(field));
        }
        v["routines"][0]["missed_run_policy"] = json!("replay_everything");
        assert!(Template::parse(&v.to_string())
            .unwrap_err()
            .contains("missed_run_policy"));
    }
}
