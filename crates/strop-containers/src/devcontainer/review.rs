use super::config::{
    ConfigurationKind, LifecycleStage, MountSpec, PortSpec, RawConfiguration, SensitiveString,
    WaitFor,
};
use super::discovery::ConfigIdentity;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectDomain {
    /// Runs on, reads from, or changes the implementing host/engine.
    Host,
    /// Runs inside or changes the selected development container.
    Container,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    ProviderResolution,
    HostInitializeCommand,
    ContainerLifecycleCommand,
    HostMount,
    EngineSocket,
    BroadHostFilesystem,
    Privileged,
    Capability,
    SecurityOption,
    Device,
    HostNetwork,
    CredentialForwarding,
    PortPublishing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EffectSeverity {
    Informational,
    Elevated,
    Critical,
}

/// A redacted, inspectable effect. `summary` never contains environment,
/// build-arg, mount-source or command values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustEffect {
    pub domain: EffectDomain,
    pub kind: EffectKind,
    pub severity: EffectSeverity,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    SshAgent,
    GpgAgent,
    CloudCredentials,
    RegistryCredentials,
    OtherCredentials,
}

/// Pure trust preview. Constructing this value performs no filesystem, Docker,
/// CLI, network or process operation.
#[derive(Debug, Clone)]
pub struct TrustReview {
    pub identity: ConfigIdentity,
    pub provider: ConfigurationKind,
    pub container_user: Option<String>,
    pub remote_user: Option<String>,
    pub workspace_folder: Option<String>,
    pub workspace_mount: Option<MountSpec>,
    pub wait_for: WaitFor,
    pub mounts: Vec<MountSpec>,
    pub credentials: Vec<CredentialKind>,
    pub ports: Vec<PortSpec>,
    pub effects: Vec<TrustEffect>,
}
impl TrustReview {
    pub fn new(identity: ConfigIdentity, configuration: &RawConfiguration) -> Self {
        let mut effects = Vec::new();
        effects.push(TrustEffect {
            domain: EffectDomain::Host,
            kind: EffectKind::ProviderResolution,
            severity: EffectSeverity::Informational,
            summary: match configuration.kind {
                ConfigurationKind::Image => {
                    "resolve the declared image through the selected engine".into()
                }
                ConfigurationKind::Dockerfile => {
                    "build/resolve the declared Dockerfile through the selected engine".into()
                }
                ConfigurationKind::Compose => {
                    "resolve the declared Compose main service and runServices".into()
                }
                ConfigurationKind::Unsupported => {
                    "configuration provider is invalid or unsupported".into()
                }
            },
        });
        for (stage, command) in configuration.lifecycle() {
            let host = stage == LifecycleStage::Initialize;
            effects.push(TrustEffect {
                domain: if host {
                    EffectDomain::Host
                } else {
                    EffectDomain::Container
                },
                kind: if host {
                    EffectKind::HostInitializeCommand
                } else {
                    EffectKind::ContainerLifecycleCommand
                },
                severity: if host {
                    EffectSeverity::Critical
                } else {
                    EffectSeverity::Elevated
                },
                summary: format!(
                    "{} configures {} command{}; command text is redacted{}",
                    stage_name(stage),
                    command.command_count(),
                    if command.command_count() == 1 {
                        ""
                    } else {
                        "s"
                    },
                    if command.is_parallel() {
                        "; named commands run in parallel"
                    } else {
                        ""
                    }
                ),
            });
        }
        for (index, mount) in configuration
            .mounts
            .iter()
            .chain(configuration.workspace_mount.iter())
            .enumerate()
        {
            let access = mount_access(mount);
            effects.push(TrustEffect {
                domain: EffectDomain::Host,
                kind: EffectKind::HostMount,
                severity: if access.socket || access.broad {
                    EffectSeverity::Critical
                } else {
                    EffectSeverity::Elevated
                },
                summary: format!(
                    "mount {} requests host/volume access; source is redacted",
                    index + 1
                ),
            });
            if access.socket {
                effects.push(TrustEffect {
                    domain: EffectDomain::Host,
                    kind: EffectKind::EngineSocket,
                    severity: EffectSeverity::Critical,
                    summary: "mount exposes a container-engine control socket".into(),
                });
            }
            if access.broad {
                effects.push(TrustEffect {
                    domain: EffectDomain::Host,
                    kind: EffectKind::BroadHostFilesystem,
                    severity: EffectSeverity::Critical,
                    summary: "mount exposes a filesystem root or home directory".into(),
                });
            }
        }
        review_run_args(&configuration.run_args, &mut effects);
        if configuration.privileged {
            push_simple(
                &mut effects,
                EffectKind::Privileged,
                EffectSeverity::Critical,
                "container requests privileged mode",
            );
        }
        for capability in &configuration.cap_add {
            effects.push(TrustEffect {
                domain: EffectDomain::Container,
                kind: EffectKind::Capability,
                severity: EffectSeverity::Critical,
                summary: format!("container requests capability {capability}"),
            });
        }
        for option in &configuration.security_opt {
            effects.push(TrustEffect {
                domain: EffectDomain::Container,
                kind: EffectKind::SecurityOption,
                severity: EffectSeverity::Elevated,
                summary: format!(
                    "container requests security option {}",
                    redact_security_option(option)
                ),
            });
        }
        for device in &configuration.devices {
            let _ = device;
            push_simple(
                &mut effects,
                EffectKind::Device,
                EffectSeverity::Critical,
                "container requests host device access",
            );
        }
        if configuration.network_mode.as_deref() == Some("host") {
            push_simple(
                &mut effects,
                EffectKind::HostNetwork,
                EffectSeverity::Critical,
                "container requests host networking",
            );
        }
        let credentials = credential_requests(configuration);
        for credential in &credentials {
            effects.push(TrustEffect {
                domain: EffectDomain::Host,
                kind: EffectKind::CredentialForwarding,
                severity: EffectSeverity::Critical,
                summary: format!("configuration may forward {}", credential_name(*credential)),
            });
        }
        let ports = configuration
            .forward_ports
            .iter()
            .chain(configuration.app_ports.iter())
            .cloned()
            .collect::<Vec<_>>();
        if !ports.is_empty() {
            effects.push(TrustEffect {
                domain: EffectDomain::Host,
                kind: EffectKind::PortPublishing,
                severity: if ports.iter().any(public_port) { EffectSeverity::Critical } else { EffectSeverity::Elevated },
                summary: format!("configuration requests {} port mapping/forwarding entries; automatic forwarding remains disabled", ports.len()),
            });
        }
        Self {
            identity,
            provider: configuration.kind,
            container_user: configuration.container_user.clone(),
            remote_user: configuration.remote_user.clone(),
            workspace_folder: configuration.workspace_folder.clone(),
            workspace_mount: configuration.workspace_mount.clone(),
            wait_for: configuration.wait_for,
            mounts: configuration.mounts.clone(),
            credentials,
            ports,
            effects,
        }
    }

    pub fn requires_consent(&self) -> bool {
        !self.effects.is_empty()
    }
    pub fn host_effects(&self) -> impl Iterator<Item = &TrustEffect> {
        self.effects
            .iter()
            .filter(|effect| effect.domain == EffectDomain::Host)
    }
    pub fn container_effects(&self) -> impl Iterator<Item = &TrustEffect> {
        self.effects
            .iter()
            .filter(|effect| effect.domain == EffectDomain::Container)
    }
}

impl fmt::Display for TrustReview {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            formatter,
            "Dev container trust review for {}",
            self.identity.config_path().display()
        )?;
        writeln!(
            formatter,
            "Configuration digest: {}",
            self.identity.digest()
        )?;
        for effect in &self.effects {
            writeln!(
                formatter,
                "- [{:?}/{:?}] {}",
                effect.domain, effect.severity, effect.summary
            )?;
        }
        Ok(())
    }
}

fn stage_name(stage: LifecycleStage) -> &'static str {
    match stage {
        LifecycleStage::Initialize => "initializeCommand (host)",
        LifecycleStage::OnCreate => "onCreateCommand",
        LifecycleStage::UpdateContent => "updateContentCommand",
        LifecycleStage::PostCreate => "postCreateCommand",
        LifecycleStage::PostStart => "postStartCommand",
        LifecycleStage::PostAttach => "postAttachCommand",
    }
}

#[derive(Default)]
struct MountAccess {
    socket: bool,
    broad: bool,
}

fn mount_access(mount: &MountSpec) -> MountAccess {
    let mut access = MountAccess::default();
    match mount {
        MountSpec::Object(object) => {
            if let Some(source) = &object.source {
                classify_source(source.expose(), &mut access);
            }
        }
        MountSpec::String(specification) => {
            let specification = specification.expose();
            let source = specification
                .split(',')
                .find_map(|part| {
                    part.strip_prefix("source=")
                        .or_else(|| part.strip_prefix("src="))
                })
                .or_else(|| {
                    if !specification.starts_with("type=") {
                        specification.split(':').next()
                    } else {
                        None
                    }
                });
            if let Some(source) = source {
                classify_source(source, &mut access);
            }
            if specification.to_ascii_lowercase().contains("ssh_auth_sock") {
                // Credential detection handles this separately.
            }
        }
    }
    access
}

fn classify_source(source: &str, access: &mut MountAccess) {
    let normalized = source.trim_end_matches('/');
    let lower = normalized.to_ascii_lowercase();
    if lower.ends_with("docker.sock")
        || lower.ends_with("containerd.sock")
        || lower.ends_with("podman.sock")
        || lower.ends_with("docker.raw")
    {
        access.socket = true;
    }
    if normalized.is_empty()
        || normalized == "/"
        || normalized == "~"
        || normalized == "$HOME"
        || normalized == "${localEnv:HOME}"
        || normalized == "${localEnv:USERPROFILE}"
        || normalized.len() == 2 && normalized.as_bytes()[1] == b':'
    {
        access.broad = true;
    }
}

fn review_run_args(arguments: &[String], effects: &mut Vec<TrustEffect>) {
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        let (option, inline) = argument
            .split_once('=')
            .map_or((argument.as_str(), None), |(a, b)| (a, Some(b)));
        let next = || arguments.get(index + 1).map(String::as_str);
        match option {
            "--privileged" => push_simple(
                effects,
                EffectKind::Privileged,
                EffectSeverity::Critical,
                "runArgs requests privileged mode",
            ),
            "--cap-add" => effects.push(TrustEffect {
                domain: EffectDomain::Container,
                kind: EffectKind::Capability,
                severity: EffectSeverity::Critical,
                summary: format!(
                    "runArgs requests capability {}",
                    inline.or_else(next).unwrap_or("<missing>")
                ),
            }),
            "--security-opt" => effects.push(TrustEffect {
                domain: EffectDomain::Container,
                kind: EffectKind::SecurityOption,
                severity: EffectSeverity::Elevated,
                summary: format!(
                    "runArgs requests security option {}",
                    redact_security_option(inline.or_else(next).unwrap_or("<missing>"))
                ),
            }),
            "--device" => push_simple(
                effects,
                EffectKind::Device,
                EffectSeverity::Critical,
                "runArgs requests host device access",
            ),
            "--network" | "--net" if matches!(inline.or_else(next), Some("host")) => push_simple(
                effects,
                EffectKind::HostNetwork,
                EffectSeverity::Critical,
                "runArgs requests host networking",
            ),
            "--mount" | "--volume" | "-v" => {
                if let Some(specification) = inline.or_else(next) {
                    let mount = MountSpec::String(SensitiveString::new(specification.to_owned()));
                    let access = mount_access(&mount);
                    effects.push(TrustEffect {
                        domain: EffectDomain::Host,
                        kind: if access.socket {
                            EffectKind::EngineSocket
                        } else {
                            EffectKind::HostMount
                        },
                        severity: if access.socket || access.broad {
                            EffectSeverity::Critical
                        } else {
                            EffectSeverity::Elevated
                        },
                        summary: "runArgs requests a mount; source is redacted".into(),
                    });
                    if access.broad {
                        push_simple(
                            effects,
                            EffectKind::BroadHostFilesystem,
                            EffectSeverity::Critical,
                            "runArgs mount exposes a filesystem root or home directory",
                        );
                    }
                }
            }
            _ => {}
        }
        index += 1;
    }
}

fn credential_requests(configuration: &RawConfiguration) -> Vec<CredentialKind> {
    let mut credentials = Vec::new();
    let mut add = |kind| {
        if !credentials.contains(&kind) {
            credentials.push(kind);
        }
    };
    for name in configuration
        .container_env
        .keys()
        .chain(configuration.remote_env.keys())
    {
        let upper = name.to_ascii_uppercase();
        if upper == "SSH_AUTH_SOCK" {
            add(CredentialKind::SshAgent);
        } else if upper.contains("GPG") {
            add(CredentialKind::GpgAgent);
        } else if upper.starts_with("AWS_")
            || upper.starts_with("AZURE_")
            || upper.starts_with("GOOGLE_")
        {
            add(CredentialKind::CloudCredentials);
        } else if upper.contains("DOCKER_AUTH") || upper.contains("REGISTRY") {
            add(CredentialKind::RegistryCredentials);
        } else if upper.contains("TOKEN")
            || upper.contains("SECRET")
            || upper.contains("PASSWORD")
            || upper.contains("CREDENTIAL")
        {
            add(CredentialKind::OtherCredentials);
        }
    }
    for mount in configuration
        .mounts
        .iter()
        .chain(configuration.workspace_mount.iter())
    {
        let value = match mount {
            MountSpec::String(value) => Some(value),
            MountSpec::Object(object) => object.source.as_ref(),
        };
        if let Some(value) = value {
            let lower = value.expose().to_ascii_lowercase();
            if lower.contains("ssh_auth_sock") || lower.contains("ssh-agent") {
                add(CredentialKind::SshAgent);
            }
            if lower.contains("gnupg") || lower.contains("gpg-agent") {
                add(CredentialKind::GpgAgent);
            }
            if lower.contains("/.aws")
                || lower.contains("/.azure")
                || lower.contains("/.config/gcloud")
            {
                add(CredentialKind::CloudCredentials);
            }
        }
    }
    for argument in &configuration.run_args {
        let lower = argument.to_ascii_lowercase();
        if lower.contains("ssh_auth_sock") || lower.contains("ssh-agent") {
            add(CredentialKind::SshAgent);
        }
        if lower.contains("gnupg") || lower.contains("gpg-agent") {
            add(CredentialKind::GpgAgent);
        }
    }
    credentials
}

fn credential_name(kind: CredentialKind) -> &'static str {
    match kind {
        CredentialKind::SshAgent => "an SSH agent",
        CredentialKind::GpgAgent => "a GPG agent",
        CredentialKind::CloudCredentials => "cloud credentials",
        CredentialKind::RegistryCredentials => "registry credentials",
        CredentialKind::OtherCredentials => "secret-bearing environment values",
    }
}

fn public_port(port: &PortSpec) -> bool {
    match port {
        PortSpec::Number(_) => false,
        PortSpec::String(mapping) => {
            let first = mapping.split(':').next().unwrap_or_default();
            mapping.contains("0.0.0.0:")
                || mapping.contains("[::]:")
                || first == "::"
                || first == "*"
        }
    }
}

fn redact_security_option(option: &str) -> &str {
    // Security options have a small grammar; never echo an arbitrary value.
    match option {
        "seccomp=unconfined" | "seccomp:unconfined" => "seccomp=unconfined",
        "apparmor=unconfined" | "apparmor:unconfined" => "apparmor=unconfined",
        "no-new-privileges" | "no-new-privileges:true" => "no-new-privileges",
        other if other.starts_with("label=") || other.starts_with("label:") => "label=<redacted>",
        _ => "<custom value redacted>",
    }
}

fn push_simple(
    effects: &mut Vec<TrustEffect>,
    kind: EffectKind,
    severity: EffectSeverity,
    summary: &str,
) {
    effects.push(TrustEffect {
        domain: EffectDomain::Container,
        kind,
        severity,
        summary: summary.into(),
    });
}
