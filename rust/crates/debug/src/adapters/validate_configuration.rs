//! Validation of debug configurations and executables.

use super::*;

impl DebugConfiguration {
    pub(crate) fn validate(
        &self,
        field: &str,
        request: DebugRequest,
        context: &DeclarationContext<'_>,
    ) -> Validation<()> {
        if self.arguments.is_none()
            && self.parameters_schema.is_none()
            && self.target_bindings.is_none()
            && self.executable.is_none()
        {
            return Err(invalid(
                field,
                "A configuration declares at least one field.",
            ));
        }
        if let Some(path) = &self.parameters_schema {
            if !is_safe_relative_path(path) || path.contains(':') {
                return Err(invalid(
                    format!("{field}.parametersSchema"),
                    "The parameters schema must be a file inside the plugin package.",
                ));
            }
        }
        let reads_executable = self
            .target_bindings
            .as_ref()
            .is_some_and(|bindings| bindings.program.as_deref() == Some(TARGET_EXECUTABLE));
        match (&self.executable, reads_executable) {
            (Some(executable), true) => {
                executable.validate(&format!("{field}.executable"), request, context)?
            }
            (Some(_), false) => {
                return Err(invalid(
                    format!("{field}.executable"),
                    "An executable lookup needs program bound to target.executable.",
                ))
            }
            (None, true) => {
                return Err(invalid(
                    format!("{field}.executable"),
                    "A program bound to target.executable needs an executable lookup.",
                ))
            }
            (None, false) => {}
        }
        let Some(bindings) = &self.target_bindings else {
            return Ok(());
        };
        let binding_field = format!("{field}.targetBindings");
        if bindings.program.is_none()
            && bindings.process_id.is_none()
            && bindings.cwd.is_none()
            && bindings.args.is_none()
            && bindings.env.is_none()
            && bindings.connection.is_none()
            && bindings.adapter_arguments.is_none()
        {
            return Err(invalid(
                &binding_field,
                "A binding list declares a binding.",
            ));
        }
        for (name, source, allowed) in [
            (
                "program",
                &bindings.program,
                &["target.path", TARGET_DIRECTORY, TARGET_EXECUTABLE][..],
            ),
            ("processId", &bindings.process_id, &["target.processID"][..]),
            ("cwd", &bindings.cwd, &["target.workingDirectory"][..]),
            ("args", &bindings.args, &["target.arguments"][..]),
            ("env", &bindings.env, &["target.environment"][..]),
        ] {
            if source
                .as_deref()
                .is_some_and(|source| !allowed.contains(&source))
            {
                return Err(invalid(
                    format!("{binding_field}.{name}"),
                    format!("{name} can only be bound to {}.", allowed.join(" or ")),
                ));
            }
        }
        if let Some(named) = &bindings.adapter_arguments {
            validate_adapter_arguments(
                &format!("{binding_field}.adapterArguments"),
                named,
                self.arguments.as_ref(),
            )?;
        }
        // `processId` names a running process and `connection` a remote
        // endpoint, which only attach has.
        if ((bindings.process_id.is_some() || bindings.connection.is_some())
            && request != DebugRequest::Attach)
            || (bindings.program.is_some() && request != DebugRequest::Launch)
        {
            return Err(invalid(
                binding_field,
                "The binding does not apply to this request.",
            ));
        }
        if let Some(connection) = &bindings.connection {
            let fixed_names: Vec<&str> = [
                ("processId", bindings.process_id.is_some()),
                ("cwd", bindings.cwd.is_some()),
            ]
            .into_iter()
            .filter_map(|(name, declared)| declared.then_some(name))
            .chain(
                bindings
                    .adapter_arguments
                    .iter()
                    .flatten()
                    .map(|(name, _)| name.as_str()),
            )
            .collect();
            connection.validate(
                &format!("{binding_field}.connection"),
                self.arguments.as_ref(),
                &fixed_names,
            )?;
        }
        Ok(())
    }
}

/// Checks the adapter-specific arguments a configuration binds: each is a
/// top-level name that nothing else sets, read from a path fact.
fn validate_adapter_arguments(
    field: &str,
    named: &BTreeMap<String, String>,
    defaults: Option<&Map<String, Value>>,
) -> Validation<()> {
    if named.is_empty() || named.len() > MAX_ADAPTER_ARGUMENTS {
        return Err(invalid(
            field,
            format!("adapterArguments binds 1-{MAX_ADAPTER_ARGUMENTS} arguments."),
        ));
    }
    for (name, source) in named {
        if !is_argument_name(name) {
            return Err(invalid(
                field,
                "An argument name is a letter or underscore followed by letters, digits, underscores, or hyphens.",
            ));
        }
        let entry = format!("{field}.{name}");
        if FIXED_BINDINGS.contains(&name.as_str()) {
            return Err(invalid(entry, format!("{name} has a binding of its own.")));
        }
        if crate::backend_start::is_host_key(name) {
            return Err(invalid(
                entry,
                "The argument belongs to the host and cannot be set by a plugin.",
            ));
        }
        if !ADAPTER_ARGUMENT_FACTS.contains(&source.as_str()) {
            return Err(invalid(
                entry,
                format!(
                    "An adapter argument can only be bound to {}.",
                    ADAPTER_ARGUMENT_FACTS.join(" or ")
                ),
            ));
        }
        // A binding only fills an argument the defaults leave out, so a
        // default of the same name would make the binding dead.
        if defaults.is_some_and(|defaults| defaults.contains_key(name)) {
            return Err(invalid(
                entry,
                "The argument is already set by the configuration's own arguments.",
            ));
        }
    }
    Ok(())
}

impl DebugExecutable {
    fn validate(
        &self,
        field: &str,
        request: DebugRequest,
        context: &DeclarationContext<'_>,
    ) -> Validation<()> {
        if request != DebugRequest::Launch {
            return Err(invalid(field, "Only a launch looks for an executable."));
        }
        if !context.declared_settings.is_file(&self.setting) {
            return Err(invalid(
                format!("{field}.setting"),
                "The setting must be a declared file setting of this plugin.",
            ));
        }
        if self.candidates.len() > MAX_EXECUTABLE_CANDIDATES {
            return Err(invalid(
                format!("{field}.candidates"),
                format!("A lookup lists at most {MAX_EXECUTABLE_CANDIDATES} candidates."),
            ));
        }
        for (index, candidate) in self.candidates.iter().enumerate() {
            if !is_executable_candidate(candidate) {
                return Err(invalid(
                    format!("{field}.candidates[{index}]"),
                    "A candidate starts with ${workspaceFolder}/ or ${fileDirectory}/, \
                     names only known variables, and has no .. or control characters.",
                ));
            }
        }
        let mut seen = BTreeSet::new();
        if !self
            .candidates
            .iter()
            .all(|candidate| seen.insert(candidate))
        {
            return Err(invalid(
                format!("{field}.candidates"),
                "A candidate is listed more than once.",
            ));
        }
        Ok(())
    }

    /// The candidates a host can look for with what it knows now, in order.
    /// One that needs the source file is skipped when there is none, which is
    /// how the settings page asks without a file open.
    pub fn expanded_candidates(
        &self,
        workspace: Option<&str>,
        target_path: Option<&str>,
    ) -> Vec<String> {
        let file = target_path.map(std::path::Path::new);
        let directory = file
            .and_then(std::path::Path::parent)
            .and_then(std::path::Path::to_str);
        let stem = file
            .and_then(std::path::Path::file_stem)
            .and_then(std::ffi::OsStr::to_str);
        let mut expanded = Vec::new();
        for candidate in &self.candidates {
            let mut path = candidate.clone();
            for (name, value) in [
                ("workspaceFolder", workspace),
                ("fileDirectory", directory),
                ("fileBasenameNoExtension", stem),
            ] {
                let variable = format!("${{{name}}}");
                if !path.contains(&variable) {
                    continue;
                }
                let Some(value) = value.filter(|value| !value.is_empty()) else {
                    path.clear();
                    break;
                };
                path = path.replace(&variable, value.trim_end_matches('/'));
            }
            if !path.is_empty() && !expanded.contains(&path) {
                expanded.push(path);
            }
        }
        expanded
    }
}

/// Whether a candidate is a path template a host may look at.
fn is_executable_candidate(candidate: &str) -> bool {
    if candidate.chars().count() > MAX_EXECUTABLE_CANDIDATE
        || candidate
            .chars()
            .any(|character| character.is_control() || character == '\\')
        || !EXECUTABLE_ROOTS
            .iter()
            .any(|root| candidate.starts_with(root))
        || candidate.split('/').any(|part| part == "..")
    {
        return false;
    }
    // Every `${` must close on a known variable; a stray one would reach the
    // file system as part of a name.
    let mut rest = candidate;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return false;
        };
        if !EXECUTABLE_VARIABLES.contains(&&after[..end]) {
            return false;
        }
        rest = &after[end + 1..];
    }
    true
}
