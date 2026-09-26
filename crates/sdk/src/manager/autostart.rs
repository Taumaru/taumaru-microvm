use super::MicroVmSdk;
use crate::domain::autostart::{
    AutostartDeleteResult, AutostartOutcome, AutostartPolicy, AutostartPolicyUpdate,
    AutostartResult, AutostartRunReport, AutostartSettings, validate_max_start_attempts,
};
use crate::domain::config::validate_vm_name;
use crate::error::SdkError;
use crate::ports::repository::StoredMicroVm;

impl MicroVmSdk {
    /// Marks an existing MicroVM for automatic start.
    ///
    /// Repeating the call with identical settings is idempotent and returns the stored policy.
    ///
    /// # Errors
    ///
    /// - [`SdkError::InvalidRequest`] for an invalid name or attempt count.
    /// - [`SdkError::NotFound`] when the MicroVM does not exist.
    /// - [`SdkError::ConfigurationConflict`] when a policy with different settings already
    ///   exists; use [`MicroVmSdk::update_autostart_policy`] to change it.
    pub async fn create_autostart_policy(
        &self,
        name: &str,
        settings: AutostartSettings,
    ) -> Result<AutostartPolicy, SdkError> {
        validate_vm_name(name)?;
        validate_max_start_attempts(settings.max_start_attempts)?;
        let name_lock = self.target_lock(&self.home.join("vms").join(name))?;
        let _name_guard = name_lock.lock().await;
        let stored = self.require_autostart_microvm(name).await?;
        let lookup_name = name.to_owned();
        let existing = self
            .run_repository(move |repository| repository.find_autostart_policy(&lookup_name))
            .await?;
        let requested = policy_from_settings(name, &settings);
        if let Some(existing) = existing {
            if existing == requested {
                return Ok(existing);
            }
            return Err(settings_conflict(&existing, &requested));
        }
        let vm_id = stored.record.id;
        self.run_repository(move |repository| repository.insert_autostart_policy(vm_id, &settings))
            .await?;
        Ok(requested)
    }

    /// Changes the fields set in `update` on an existing autostart policy.
    ///
    /// An empty update returns the stored policy unchanged.
    ///
    /// # Errors
    ///
    /// - [`SdkError::InvalidRequest`] for an invalid name or attempt count.
    /// - [`SdkError::NotFound`] when the MicroVM or its autostart policy does not exist.
    pub async fn update_autostart_policy(
        &self,
        name: &str,
        update: AutostartPolicyUpdate,
    ) -> Result<AutostartPolicy, SdkError> {
        validate_vm_name(name)?;
        if let Some(attempts) = update.max_start_attempts {
            validate_max_start_attempts(attempts)?;
        }
        let name_lock = self.target_lock(&self.home.join("vms").join(name))?;
        let _name_guard = name_lock.lock().await;
        let stored = self.require_autostart_microvm(name).await?;
        let lookup_name = name.to_owned();
        let existing = self
            .run_repository(move |repository| repository.find_autostart_policy(&lookup_name))
            .await?
            .ok_or_else(|| policy_not_found(name))?;
        let settings = AutostartSettings {
            enabled: update.enabled.unwrap_or(existing.enabled),
            max_start_attempts: update
                .max_start_attempts
                .unwrap_or(existing.max_start_attempts),
        };
        let updated = policy_from_settings(name, &settings);
        if updated == existing {
            return Ok(existing);
        }
        let vm_id = stored.record.id;
        self.run_repository(move |repository| repository.update_autostart_policy(vm_id, &settings))
            .await?;
        Ok(updated)
    }

    /// Removes the autostart policy of a MicroVM. The MicroVM itself is untouched.
    ///
    /// Idempotent: a MicroVM without a policy returns `removed: false`.
    ///
    /// # Errors
    ///
    /// - [`SdkError::InvalidRequest`] for an invalid name.
    /// - [`SdkError::NotFound`] when the MicroVM does not exist.
    pub async fn delete_autostart_policy(
        &self,
        name: &str,
    ) -> Result<AutostartDeleteResult, SdkError> {
        validate_vm_name(name)?;
        let name_lock = self.target_lock(&self.home.join("vms").join(name))?;
        let _name_guard = name_lock.lock().await;
        let stored = self.require_autostart_microvm(name).await?;
        let vm_id = stored.record.id;
        let removed = self
            .run_repository(move |repository| repository.delete_autostart_policy(vm_id))
            .await?;
        Ok(AutostartDeleteResult {
            name: name.to_owned(),
            removed,
        })
    }

    /// Returns the autostart policy of a MicroVM, or `None` when it has none.
    ///
    /// # Errors
    ///
    /// [`SdkError::InvalidRequest`] for an invalid name, or a persistence error.
    pub async fn autostart_policy(&self, name: &str) -> Result<Option<AutostartPolicy>, SdkError> {
        validate_vm_name(name)?;
        let lookup_name = name.to_owned();
        self.run_repository(move |repository| repository.find_autostart_policy(&lookup_name))
            .await
    }

    /// Lists every stored autostart policy, ordered by MicroVM name.
    pub async fn list_autostart_policies(&self) -> Result<Vec<AutostartPolicy>, SdkError> {
        self.run_repository(|repository| repository.list_autostart_policies())
            .await
    }

    /// Starts every MicroVM with an enabled autostart policy, typically at host boot.
    ///
    /// Machines are processed sequentially in name order through
    /// [`MicroVmSdk::start_microvm`], which already treats a live machine as success. Each
    /// enabled machine gets up to its `max_start_attempts`, with a linearly growing pause
    /// between attempts. Paused policies are reported without being started. A failure of one
    /// machine never prevents the others from starting; inspect
    /// [`AutostartRunReport::has_failures`] for the aggregate result.
    ///
    /// # Errors
    ///
    /// Only a failure to read the stored policies is returned as an error.
    pub async fn start_autostart_microvms(&self) -> Result<AutostartRunReport, SdkError> {
        let policies = self.list_autostart_policies().await?;
        let mut report = AutostartRunReport::default();
        for policy in policies {
            let outcome = if policy.enabled {
                self.start_with_attempts(&policy).await
            } else {
                AutostartOutcome {
                    name: policy.name,
                    attempts: 0,
                    result: AutostartResult::Paused,
                }
            };
            report.outcomes.push(outcome);
        }
        Ok(report)
    }

    async fn start_with_attempts(&self, policy: &AutostartPolicy) -> AutostartOutcome {
        let mut attempt = 1;
        loop {
            match self.start_microvm(&policy.name).await {
                Ok(started) => {
                    return AutostartOutcome {
                        name: policy.name.clone(),
                        attempts: attempt,
                        result: AutostartResult::Started(Box::new(started)),
                    };
                }
                Err(error) if attempt >= policy.max_start_attempts => {
                    return AutostartOutcome {
                        name: policy.name.clone(),
                        attempts: attempt,
                        result: AutostartResult::Failed(error),
                    };
                }
                Err(_) => {
                    tokio::time::sleep(self.autostart_retry_backoff * attempt).await;
                    attempt += 1;
                }
            }
        }
    }

    async fn require_autostart_microvm(&self, name: &str) -> Result<StoredMicroVm, SdkError> {
        let lookup_name = name.to_owned();
        self.run_repository(move |repository| repository.find_microvm(&lookup_name))
            .await?
            .ok_or_else(|| SdkError::NotFound {
                kind: "MicroVM".to_owned(),
                id: name.to_owned(),
            })
    }
}

fn policy_from_settings(name: &str, settings: &AutostartSettings) -> AutostartPolicy {
    AutostartPolicy {
        name: name.to_owned(),
        enabled: settings.enabled,
        max_start_attempts: settings.max_start_attempts,
    }
}

fn policy_not_found(name: &str) -> SdkError {
    SdkError::NotFound {
        kind: "autostart policy".to_owned(),
        id: name.to_owned(),
    }
}

fn settings_conflict(existing: &AutostartPolicy, requested: &AutostartPolicy) -> SdkError {
    let (field, existing_value, requested_value) = if existing.enabled != requested.enabled {
        (
            "enabled",
            existing.enabled.to_string(),
            requested.enabled.to_string(),
        )
    } else {
        (
            "max_start_attempts",
            existing.max_start_attempts.to_string(),
            requested.max_start_attempts.to_string(),
        )
    };
    SdkError::ConfigurationConflict {
        name: existing.name.clone(),
        field: format!("autostart.{field}"),
        existing: existing_value,
        requested: requested_value,
    }
}
