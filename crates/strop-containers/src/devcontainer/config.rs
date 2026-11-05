use super::jsonc;
use super::DevcontainerError;
mod fields;
use fields::known_fields;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// A configuration value whose contents must not appear in Debug, Display,
/// ordinary diagnostic logs or trust previews. Access is explicit for the
/// CLI/redaction boundary; serde is used only by the explicit full-content
/// replay outcome boundary, never by redacted operational output.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SensitiveString(String);
impl SensitiveString {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for SensitiveString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}
impl fmt::Display for SensitiveString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConfigurationKind {
    Image,
    Dockerfile,
    Compose,
    /// Missing or conflicting providers. It remains reviewable but lifecycle
    /// operations refuse it rather than approximating it as `docker run`.
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigIssueKind {
    InvalidType,
    ConflictingProviders,
    MissingProperty,
    UnsupportedProperty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigIssue {
    pub path: String,
    pub kind: ConfigIssueKind,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawBuild {
    pub dockerfile: Option<String>,
    pub context: Option<String>,
    pub target: Option<String>,
    pub cache_from: Option<String>,
    pub args: BTreeMap<String, SensitiveString>,
    pub options: Vec<String>,
}

/// Lifecycle command shapes are data only. Even Debug deliberately hides
/// command text because commands can contain inline credentials.
#[derive(Clone, PartialEq, Eq)]
pub enum LifecycleCommand {
    Shell(SensitiveString),
    Argv(Vec<SensitiveString>),
    Parallel(BTreeMap<String, LifecycleCommand>),
}
impl LifecycleCommand {
    pub fn command_count(&self) -> usize {
        match self {
            Self::Shell(_) | Self::Argv(_) => 1,
            Self::Parallel(commands) => commands.values().map(Self::command_count).sum(),
        }
    }
    pub fn is_parallel(&self) -> bool {
        matches!(self, Self::Parallel(_))
    }
}
impl fmt::Debug for LifecycleCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shell(_) => formatter.write_str("Shell(<redacted>)"),
            Self::Argv(arguments) => formatter
                .debug_tuple("Argv")
                .field(&format_args!("<{} redacted arguments>", arguments.len()))
                .finish(),
            Self::Parallel(commands) => formatter
                .debug_struct("Parallel")
                .field("names", &commands.keys().collect::<Vec<_>>())
                .field("commands", &commands.len())
                .finish(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LifecycleStage {
    Initialize,
    OnCreate,
    UpdateContent,
    PostCreate,
    PostStart,
    PostAttach,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum WaitFor {
    OnCreateCommand,
    #[default]
    UpdateContentCommand,
    PostCreateCommand,
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MountSpec {
    String(SensitiveString),
    Object(MountObject),
}
impl MountSpec {
    pub fn object(&self) -> Option<&MountObject> {
        match self {
            Self::Object(object) => Some(object),
            Self::String(_) => None,
        }
    }
    pub fn string(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value.expose()),
            Self::Object(_) => None,
        }
    }
}
impl fmt::Debug for MountSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(_) => formatter.write_str("String(<redacted>)"),
            Self::Object(object) => object.fmt(formatter),
        }
    }
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MountObject {
    pub kind: Option<String>,
    pub source: Option<SensitiveString>,
    pub target: String,
    pub options: BTreeMap<String, Value>,
}
impl fmt::Debug for MountObject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MountObject")
            .field("kind", &self.kind)
            .field("source", &self.source)
            .field("target", &self.target)
            .field("options", &self.options.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum PortSpec {
    Number(u64),
    String(String),
}
impl fmt::Debug for PortSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(port) => port.fmt(formatter),
            Self::String(mapping) => formatter.debug_tuple("Mapping").field(mapping).finish(),
        }
    }
}

/// Typed view of the security/lifecycle-relevant specification fields. The
/// original JSON object remains available for inspection and for fields that
/// this crate deliberately leaves to the reference CLI.
#[derive(Clone)]
pub struct RawConfiguration {
    pub name: Option<String>,
    pub kind: ConfigurationKind,
    pub image: Option<String>,
    pub dockerfile: Option<String>,
    pub build: Option<RawBuild>,
    pub compose_files: Vec<String>,
    pub service: Option<String>,
    pub run_services: Vec<String>,
    pub workspace_folder: Option<String>,
    pub workspace_mount: Option<MountSpec>,
    pub container_user: Option<String>,
    pub remote_user: Option<String>,
    pub initialize_command: Option<LifecycleCommand>,
    pub on_create_command: Option<LifecycleCommand>,
    pub update_content_command: Option<LifecycleCommand>,
    pub post_create_command: Option<LifecycleCommand>,
    pub post_start_command: Option<LifecycleCommand>,
    pub post_attach_command: Option<LifecycleCommand>,
    pub wait_for: WaitFor,
    pub mounts: Vec<MountSpec>,
    pub container_env: BTreeMap<String, SensitiveString>,
    pub remote_env: BTreeMap<String, SensitiveString>,
    pub privileged: bool,
    pub cap_add: Vec<String>,
    pub security_opt: Vec<String>,
    pub devices: Vec<MountSpec>,
    pub network_mode: Option<String>,
    pub run_args: Vec<String>,
    pub forward_ports: Vec<PortSpec>,
    pub app_ports: Vec<PortSpec>,
    pub features: Option<Value>,
    pub shutdown_action: Option<String>,
    pub issues: Vec<ConfigIssue>,
    source: Value,
    additional: BTreeMap<String, Value>,
}

impl fmt::Debug for RawConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RawConfiguration")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field(
                "users",
                &(self.container_user.is_some(), self.remote_user.is_some()),
            )
            .field("lifecycle_stages", &self.lifecycle().len())
            .field("mounts", &self.mounts.len())
            .field(
                "container_env_keys",
                &self.container_env.keys().collect::<Vec<_>>(),
            )
            .field(
                "remote_env_keys",
                &self.remote_env.keys().collect::<Vec<_>>(),
            )
            .field("privileged", &self.privileged)
            .field("issues", &self.issues)
            .finish_non_exhaustive()
    }
}

impl RawConfiguration {
    pub fn parse_jsonc(bytes: &[u8], path: &Path, limit: usize) -> Result<Self, DevcontainerError> {
        Self::from_value(jsonc::parse_bytes(bytes, path, limit)?)
    }

    /// Build the same typed view from reference-CLI JSON output. Unsupported
    /// field shapes add issues and retain their original values in `source`.
    pub fn from_value(source: Value) -> Result<Self, DevcontainerError> {
        let object = source.as_object().ok_or(DevcontainerError::InvalidRoot)?;
        let mut parser = Parser { issues: Vec::new() };
        let image = parser.string(object, "image");
        let top_dockerfile = parser.string(object, "dockerFile");
        let build = parser.build(object);
        let compose_files = parser.string_list(object, "dockerComposeFile");
        let service = parser.string(object, "service");
        let provider_count = usize::from(image.is_some())
            + usize::from(top_dockerfile.is_some() || build.is_some())
            + usize::from(!compose_files.is_empty() || service.is_some());
        let kind = match (
            image.is_some(),
            top_dockerfile.is_some() || build.is_some(),
            !compose_files.is_empty(),
        ) {
            (true, false, false) => ConfigurationKind::Image,
            (false, true, false) => ConfigurationKind::Dockerfile,
            (false, false, true) => ConfigurationKind::Compose,
            _ => {
                if provider_count != 1 {
                    parser.issue(
                        "<root>",
                        if provider_count == 0 {
                            ConfigIssueKind::MissingProperty
                        } else {
                            ConfigIssueKind::ConflictingProviders
                        },
                        "select exactly one of image, Dockerfile/build, or dockerComposeFile",
                    );
                }
                ConfigurationKind::Unsupported
            }
        };
        if compose_files.is_empty() && service.is_some() {
            parser.issue(
                "dockerComposeFile",
                ConfigIssueKind::MissingProperty,
                "a Compose main service requires dockerComposeFile",
            );
        }
        if kind == ConfigurationKind::Compose && service.is_none() {
            parser.issue(
                "service",
                ConfigIssueKind::MissingProperty,
                "Compose requires a main service",
            );
        }
        let mut dockerfile = top_dockerfile;
        if dockerfile.is_none() {
            dockerfile = build.as_ref().and_then(|build| build.dockerfile.clone());
        }
        let wait_for = match parser.string(object, "waitFor").as_deref() {
            None => WaitFor::default(),
            Some("onCreateCommand") => WaitFor::OnCreateCommand,
            Some("updateContentCommand") => WaitFor::UpdateContentCommand,
            Some("postCreateCommand") => WaitFor::PostCreateCommand,
            Some(_) => {
                parser.issue(
                    "waitFor",
                    ConfigIssueKind::InvalidType,
                    "expected a lifecycle command name",
                );
                WaitFor::default()
            }
        };
        let mounts = parser.mounts(object, "mounts");
        let devices = parser.mounts(object, "devices");
        let known = known_fields();
        let additional = object
            .iter()
            .filter(|(key, _)| !known.contains(key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();
        for key in additional.keys() {
            parser.issue(
                key,
                ConfigIssueKind::UnsupportedProperty,
                "retained for reference-CLI handling; not modeled by the passive review",
            );
        }
        Ok(Self {
            name: parser.string(object, "name"),
            kind,
            image,
            dockerfile,
            build,
            compose_files,
            service,
            run_services: parser.string_list(object, "runServices"),
            workspace_folder: parser.string(object, "workspaceFolder"),
            workspace_mount: parser.mount(object, "workspaceMount"),
            container_user: parser.string(object, "containerUser"),
            remote_user: parser.string(object, "remoteUser"),
            initialize_command: parser.command(object, "initializeCommand"),
            on_create_command: parser.command(object, "onCreateCommand"),
            update_content_command: parser.command(object, "updateContentCommand"),
            post_create_command: parser.command(object, "postCreateCommand"),
            post_start_command: parser.command(object, "postStartCommand"),
            post_attach_command: parser.command(object, "postAttachCommand"),
            wait_for,
            mounts,
            container_env: parser.environment(object, "containerEnv"),
            remote_env: parser.environment(object, "remoteEnv"),
            privileged: parser.boolean(object, "privileged").unwrap_or(false),
            cap_add: parser.string_list(object, "capAdd"),
            security_opt: parser.string_list(object, "securityOpt"),
            devices,
            network_mode: parser.string(object, "networkMode"),
            run_args: parser.string_list(object, "runArgs"),
            forward_ports: parser.ports(object, "forwardPorts"),
            app_ports: parser.ports(object, "appPort"),
            features: object.get("features").cloned(),
            shutdown_action: parser.string(object, "shutdownAction"),
            issues: parser.issues,
            source: source.clone(),
            additional,
        })
    }

    pub fn source(&self) -> &Value {
        &self.source
    }
    pub fn additional(&self) -> &BTreeMap<String, Value> {
        &self.additional
    }
    pub fn has_blocking_issues(&self) -> bool {
        self.kind == ConfigurationKind::Unsupported
            || self.issues.iter().any(|issue| {
                matches!(
                    issue.kind,
                    ConfigIssueKind::InvalidType
                        | ConfigIssueKind::ConflictingProviders
                        | ConfigIssueKind::MissingProperty
                )
            })
    }
    pub fn lifecycle(&self) -> Vec<(LifecycleStage, &LifecycleCommand)> {
        [
            (LifecycleStage::Initialize, self.initialize_command.as_ref()),
            (LifecycleStage::OnCreate, self.on_create_command.as_ref()),
            (
                LifecycleStage::UpdateContent,
                self.update_content_command.as_ref(),
            ),
            (
                LifecycleStage::PostCreate,
                self.post_create_command.as_ref(),
            ),
            (LifecycleStage::PostStart, self.post_start_command.as_ref()),
            (
                LifecycleStage::PostAttach,
                self.post_attach_command.as_ref(),
            ),
        ]
        .into_iter()
        .filter_map(|(stage, command)| command.map(|command| (stage, command)))
        .collect()
    }
    pub(crate) fn sensitive_values(&self) -> Vec<String> {
        let mut values = Vec::new();
        for value in self.container_env.values().chain(self.remote_env.values()) {
            if !value.expose().is_empty() {
                values.push(value.expose().to_owned());
            }
        }
        if let Some(build) = &self.build {
            values.extend(
                build
                    .args
                    .values()
                    .filter(|value| !value.expose().is_empty())
                    .map(|value| value.expose().to_owned()),
            );
        }
        for command in self.lifecycle().into_iter().map(|(_, command)| command) {
            collect_command_text(command, &mut values);
        }
        values.sort_by_key(|value| std::cmp::Reverse(value.len()));
        values.dedup();
        values
    }
}

fn collect_command_text(command: &LifecycleCommand, output: &mut Vec<String>) {
    match command {
        LifecycleCommand::Shell(text) => output.push(text.expose().to_owned()),
        LifecycleCommand::Argv(arguments) => {
            output.extend(
                arguments
                    .iter()
                    .map(|argument| argument.expose().to_owned()),
            );
        }
        LifecycleCommand::Parallel(commands) => {
            for command in commands.values() {
                collect_command_text(command, output);
            }
        }
    }
}

struct Parser {
    issues: Vec<ConfigIssue>,
}
impl Parser {
    fn issue(&mut self, path: &str, kind: ConfigIssueKind, detail: &str) {
        self.issues.push(ConfigIssue {
            path: path.into(),
            kind,
            detail: detail.into(),
        });
    }
    fn string(&mut self, object: &Map<String, Value>, key: &str) -> Option<String> {
        match object.get(key) {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => Some(value.clone()),
            Some(value) => {
                self.issue(
                    key,
                    ConfigIssueKind::InvalidType,
                    &format!("expected string, found {}", shape(value)),
                );
                None
            }
        }
    }
    fn boolean(&mut self, object: &Map<String, Value>, key: &str) -> Option<bool> {
        match object.get(key) {
            None | Some(Value::Null) => None,
            Some(Value::Bool(value)) => Some(*value),
            Some(value) => {
                self.issue(
                    key,
                    ConfigIssueKind::InvalidType,
                    &format!("expected boolean, found {}", shape(value)),
                );
                None
            }
        }
    }
    fn string_list(&mut self, object: &Map<String, Value>, key: &str) -> Vec<String> {
        match object.get(key) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::String(value)) => vec![value.clone()],
            Some(Value::Array(values)) => values
                .iter()
                .enumerate()
                .filter_map(|(index, value)| {
                    if let Value::String(value) = value {
                        Some(value.clone())
                    } else {
                        self.issue(
                            &format!("{key}[{index}]"),
                            ConfigIssueKind::InvalidType,
                            "expected string",
                        );
                        None
                    }
                })
                .collect(),
            Some(value) => {
                self.issue(
                    key,
                    ConfigIssueKind::InvalidType,
                    &format!("expected string or string array, found {}", shape(value)),
                );
                Vec::new()
            }
        }
    }
    fn environment(
        &mut self,
        object: &Map<String, Value>,
        key: &str,
    ) -> BTreeMap<String, SensitiveString> {
        let mut output = BTreeMap::new();
        match object.get(key) {
            None | Some(Value::Null) => {}
            Some(Value::Object(values)) => {
                for (name, value) in values {
                    match value {
                        Value::String(value) => {
                            output.insert(name.clone(), SensitiveString(value.clone()));
                        }
                        Value::Null => {
                            output.insert(name.clone(), SensitiveString(String::new()));
                        }
                        _ => self.issue(
                            &format!("{key}.{name}"),
                            ConfigIssueKind::InvalidType,
                            "expected string or null",
                        ),
                    }
                }
            }
            Some(value) => self.issue(
                key,
                ConfigIssueKind::InvalidType,
                &format!("expected object, found {}", shape(value)),
            ),
        }
        output
    }
    fn command(&mut self, object: &Map<String, Value>, key: &str) -> Option<LifecycleCommand> {
        self.command_value(key, object.get(key)?)
    }
    fn command_value(&mut self, path: &str, value: &Value) -> Option<LifecycleCommand> {
        match value {
            Value::Null => None,
            Value::String(command) => {
                Some(LifecycleCommand::Shell(SensitiveString(command.clone())))
            }
            Value::Array(arguments) => {
                let mut output = Vec::new();
                for (index, argument) in arguments.iter().enumerate() {
                    if let Value::String(argument) = argument {
                        output.push(SensitiveString(argument.clone()));
                    } else {
                        self.issue(
                            &format!("{path}[{index}]"),
                            ConfigIssueKind::InvalidType,
                            "expected command argument string",
                        );
                    }
                }
                Some(LifecycleCommand::Argv(output))
            }
            Value::Object(commands) => {
                let mut output = BTreeMap::new();
                for (name, command) in commands {
                    if let Some(command) = self.command_value(&format!("{path}.{name}"), command) {
                        output.insert(name.clone(), command);
                    }
                }
                Some(LifecycleCommand::Parallel(output))
            }
            _ => {
                self.issue(
                    path,
                    ConfigIssueKind::InvalidType,
                    "expected command string, argv array, or named command object",
                );
                None
            }
        }
    }
    fn build(&mut self, object: &Map<String, Value>) -> Option<RawBuild> {
        let value = object.get("build")?;
        if value.is_null() {
            return None;
        }
        let Some(build) = value.as_object() else {
            self.issue(
                "build",
                ConfigIssueKind::InvalidType,
                "expected build object",
            );
            return None;
        };
        let args = match build.get("args") {
            Some(Value::Object(values)) => {
                let mut args = BTreeMap::new();
                for (name, value) in values {
                    match value {
                        Value::String(value) => {
                            args.insert(name.clone(), SensitiveString(value.clone()));
                        }
                        Value::Number(value) => {
                            args.insert(name.clone(), SensitiveString(value.to_string()));
                        }
                        Value::Bool(value) => {
                            args.insert(name.clone(), SensitiveString(value.to_string()));
                        }
                        Value::Null => {
                            args.insert(name.clone(), SensitiveString(String::new()));
                        }
                        _ => self.issue(
                            &format!("build.args.{name}"),
                            ConfigIssueKind::InvalidType,
                            "expected scalar build argument",
                        ),
                    }
                }
                args
            }
            None | Some(Value::Null) => BTreeMap::new(),
            Some(_) => {
                self.issue(
                    "build.args",
                    ConfigIssueKind::InvalidType,
                    "expected object",
                );
                BTreeMap::new()
            }
        };
        Some(RawBuild {
            dockerfile: self.string(build, "dockerfile"),
            context: self.string(build, "context"),
            target: self.string(build, "target"),
            cache_from: self.string(build, "cacheFrom"),
            args,
            options: self.string_list(build, "options"),
        })
    }
    fn mount(&mut self, object: &Map<String, Value>, key: &str) -> Option<MountSpec> {
        self.mount_value(key, object.get(key)?)
    }
    fn mount_value(&mut self, path: &str, value: &Value) -> Option<MountSpec> {
        match value {
            Value::Null => None,
            Value::String(value) => Some(MountSpec::String(SensitiveString(value.clone()))),
            Value::Object(object) => {
                let target = self
                    .string(object, "target")
                    .or_else(|| self.string(object, "dst"))
                    .or_else(|| self.string(object, "destination"));
                if target.is_none() {
                    self.issue(
                        &format!("{path}.target"),
                        ConfigIssueKind::MissingProperty,
                        "mount object requires target",
                    );
                }
                let known = ["type", "source", "src", "target", "dst", "destination"];
                let options = object
                    .iter()
                    .filter(|(key, _)| !known.contains(&key.as_str()))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                Some(MountSpec::Object(MountObject {
                    kind: self.string(object, "type"),
                    source: self
                        .string(object, "source")
                        .or_else(|| self.string(object, "src"))
                        .map(SensitiveString),
                    target: target.unwrap_or_default(),
                    options,
                }))
            }
            _ => {
                self.issue(
                    path,
                    ConfigIssueKind::InvalidType,
                    "expected mount string or object",
                );
                None
            }
        }
    }
    fn mounts(&mut self, object: &Map<String, Value>, key: &str) -> Vec<MountSpec> {
        match object.get(key) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(values)) => values
                .iter()
                .enumerate()
                .filter_map(|(index, value)| self.mount_value(&format!("{key}[{index}]"), value))
                .collect(),
            Some(value @ Value::String(_)) | Some(value @ Value::Object(_)) => {
                self.mount_value(key, value).into_iter().collect()
            }
            Some(_) => {
                self.issue(
                    key,
                    ConfigIssueKind::InvalidType,
                    "expected mount or mount array",
                );
                Vec::new()
            }
        }
    }
    fn ports(&mut self, object: &Map<String, Value>, key: &str) -> Vec<PortSpec> {
        fn convert(parser: &mut Parser, path: &str, value: &Value) -> Option<PortSpec> {
            match value {
                Value::Number(number) => number
                    .as_u64()
                    .filter(|port| *port <= 65535)
                    .map(PortSpec::Number)
                    .or_else(|| {
                        parser.issue(
                            path,
                            ConfigIssueKind::InvalidType,
                            "expected a TCP/UDP port number",
                        );
                        None
                    }),
                Value::String(mapping) => Some(PortSpec::String(mapping.clone())),
                _ => {
                    parser.issue(
                        path,
                        ConfigIssueKind::InvalidType,
                        "expected port number or mapping string",
                    );
                    None
                }
            }
        }
        match object.get(key) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(values)) => values
                .iter()
                .enumerate()
                .filter_map(|(index, value)| convert(self, &format!("{key}[{index}]"), value))
                .collect(),
            Some(value) => convert(self, key, value).into_iter().collect(),
        }
    }
}

fn shape(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
