use std::fmt;

use inquire::{Confirm, Select, Text};
use taumaru_microvm::{
    CreateMicroVmRequest, CreationProgress, Distribution, DistributionImage, DownloadCancellation,
    Kernel, MicroVmSdk, MicroVmState, SdkError,
};

use super::download::{
    ArtifactClient, OperationResult, RegistryCatalog, SdkArtifactClient, binary_package_size,
    call_with_signal, checked_size_add, format_image_bytes, host_architecture, load_catalog,
    parse_image_selections, prompt_render_config, same_architecture,
};
use crate::cli::NewArgs;
use crate::context::{CliContext, TerminalCapabilities};
use crate::error::CliError;

const BYTES_PER_GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const BYTES_PER_MIB_F64: f64 = 1024.0 * 1024.0;

#[derive(Clone, Debug)]
pub(crate) struct NewVmRequest {
    pub(crate) name: String,
    pub(crate) distribution_id: String,
    pub(crate) image_id: String,
    pub(crate) disk_size_bytes: u64,
    pub(crate) memory_bytes: u64,
    pub(crate) vcpu_count: u32,
    pub(crate) expose_on_lan: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct NewImageChoice {
    pub(crate) distribution: Distribution,
    pub(crate) image: DistributionImage,
    pub(crate) kernel: Kernel,
    pub(crate) expected_bytes: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct NewProvisioningPlan {
    pub(crate) runtime_packages: Vec<taumaru_microvm::BinaryPackage>,
    pub(crate) kernel: Kernel,
    pub(crate) provision_bytes: u64,
}

pub(crate) fn resolve_name(
    positional: Option<&str>,
    explicit: Option<&str>,
) -> Result<String, CliError> {
    match (positional, explicit) {
        (Some(positional), Some(explicit)) if positional != explicit => Err(CliError::creation(
            "Machine name mismatch",
            "positional name {positional:?} differs from --name {explicit:?}",
            "Pass one name",
        )),
        (Some(positional), _) => validate_name(positional),
        (None, Some(explicit)) => validate_name(explicit),
        (None, None) => Err(CliError::missing_value(
            "machine name",
            "--name <NAME>",
            "Run microvm new web-01 --non-interactive --image <distribution>=<image> --disk-gb 20 --memory 2GB --vcpus 2",
        )),
    }
}

pub(crate) fn validate_name(name: &str) -> Result<String, CliError> {
    if name.len() > 64 {
        return Err(invalid_name(name));
    }
    let valid = !name.is_empty()
        && name.is_ascii()
        && name.bytes().enumerate().all(|(index, byte)| {
            if index == 0 {
                byte.is_ascii_alphanumeric()
            } else {
                byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
            }
        });
    if valid {
        Ok(name.to_owned())
    } else {
        Err(invalid_name(name))
    }
}

fn new_prompt_error(error: inquire::InquireError) -> CliError {
    let message = error.to_string();
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("cancel") || normalized.contains("interrupt") {
        CliError::cancelled()
    } else {
        CliError::creation(
            "MicroVM creation input could not be completed",
            message,
            "Check terminal input and try again",
        )
    }
}

fn invalid_name(name: &str) -> CliError {
    CliError::creation(
        "Machine name is invalid",
        format!(
            "name {name:?} must be 1-64 ASCII characters, start with a letter or digit, and contain only letters, digits, '-' or '_'"
        ),
        "Choose a path-friendly name or pass --name <NAME>",
    )
}

pub(crate) fn parse_disk_gb(value: &str) -> Result<u64, CliError> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let amount_text = lower.strip_suffix("gb").map(str::trim).unwrap_or(trimmed);
    let amount: f64 = amount_text.parse().unwrap_or(f64::NAN);
    if !amount.is_finite() || amount <= 0.0 {
        return Err(CliError::creation(
            "Disk size is invalid",
            format!("disk size {value:?} must be a positive number in GB"),
            "Pass --disk-gb <GB> with a positive number",
        ));
    }
    let bytes = (amount * BYTES_PER_GIB).ceil();
    if !bytes.is_finite() || bytes > u64::MAX as f64 {
        return Err(CliError::creation(
            "Disk size is invalid",
            format!("disk size {value:?} exceeds the supported size range"),
            "Choose a smaller --disk-gb value",
        ));
    }
    Ok(bytes as u64)
}

pub(crate) fn parse_memory(value: &str) -> Result<u64, CliError> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let (amount_text, multiplier) = if let Some(amount) = lower.strip_suffix("gb") {
        (amount.trim(), BYTES_PER_GIB)
    } else if let Some(amount) = lower.strip_suffix("mb") {
        (amount.trim(), BYTES_PER_MIB_F64)
    } else {
        (trimmed, BYTES_PER_MIB_F64)
    };
    let amount: f64 = amount_text.parse().unwrap_or(f64::NAN);
    if amount_text.is_empty() || !amount.is_finite() || amount <= 0.0 {
        return Err(invalid_memory(value));
    }
    let bytes = (amount * multiplier).ceil();
    if !bytes.is_finite() || bytes > u64::MAX as f64 {
        return Err(CliError::creation(
            "Memory size is invalid",
            format!("memory size {value:?} exceeds the supported size range"),
            "Choose a smaller --memory value",
        ));
    }
    Ok(bytes as u64)
}

fn invalid_memory(value: &str) -> CliError {
    CliError::creation(
        "Memory size is invalid",
        format!("memory size {value:?} must use xMB or xGB, for example 512MB or 1.5GB"),
        "Pass --memory <xMB|xGB> with a positive amount",
    )
}

pub(crate) fn parse_vcpus(value: &str) -> Result<u32, CliError> {
    let trimmed = value.trim();
    match trimmed.parse::<u32>() {
        Ok(count) if count > 0 => Ok(count),
        _ => Err(CliError::creation(
            "VCPU count is invalid",
            format!("vCPU count {value:?} must be a positive integer"),
            "Pass --vcpus <N> with a positive integer",
        )),
    }
}

pub(crate) fn parse_single_image(value: &str) -> Result<(String, String), CliError> {
    let selections = parse_image_selections(std::slice::from_ref(&value.to_owned()))?;
    Ok(selections.into_iter().next().expect("one selection parsed"))
}

pub(crate) fn check_disk_minimum(
    disk_size_bytes: u64,
    image_size_bytes: u64,
) -> Result<(), CliError> {
    if disk_size_bytes < image_size_bytes {
        return Err(CliError::creation(
            "Disk size is too small",
            format!(
                "requested {} is smaller than the image minimum of {}",
                format_gb(disk_size_bytes),
                format_gb(image_size_bytes)
            ),
            format!(
                "choose --disk-gb of at least {}",
                format_gb(image_size_bytes)
            ),
        ));
    }
    Ok(())
}

pub(crate) fn check_memory_minimum(memory_bytes: u64, minimum_bytes: u64) -> Result<(), CliError> {
    if memory_bytes < minimum_bytes {
        return Err(CliError::creation(
            "Memory size is too small",
            format!(
                "requested {} is smaller than the distribution minimum of {}",
                format_mb_gb(memory_bytes),
                format_mb_gb(minimum_bytes)
            ),
            format!(
                "choose --memory of at least {}",
                format_mb_gb(minimum_bytes)
            ),
        ));
    }
    Ok(())
}

pub(crate) fn check_vcpu_minimum(
    vcpus: u32,
    minimum: u32,
    distribution: &str,
) -> Result<(), CliError> {
    if vcpus < minimum {
        return Err(CliError::creation(
            "VCPU count is too small",
            format!(
                "requested {vcpus} vCPUs is fewer than the distribution {distribution} minimum of {minimum}"
            ),
            format!("choose --vcpus of at least {minimum}"),
        ));
    }
    Ok(())
}

pub(crate) fn format_gb(bytes: u64) -> String {
    let value = bytes as f64 / BYTES_PER_GIB;
    if value.fract() == 0.0 {
        format!("{} GB", value as u64)
    } else {
        format!("{value:.1} GB")
    }
}

pub(crate) fn format_mb_gb(bytes: u64) -> String {
    if bytes >= BYTES_PER_GIB as u64 && bytes.is_multiple_of(BYTES_PER_GIB as u64) {
        return format_gb(bytes);
    }
    let value = bytes as f64 / BYTES_PER_MIB_F64;
    if value.fract() == 0.0 {
        format!("{} MB", value as u64)
    } else {
        format!("{value:.1} MB")
    }
}

fn minimum_memory_bytes(min_memory_mb: u64) -> Result<u64, CliError> {
    min_memory_mb.checked_mul(1024 * 1024).ok_or_else(|| {
        CliError::creation(
            "Distribution requirements are invalid",
            "minimum memory exceeds the supported range",
            "Retry after the registry publishes valid requirements",
        )
    })
}

pub(crate) fn resolve_choice(
    catalog: &RegistryCatalog,
    distribution_id: &str,
    image_id: &str,
) -> Result<NewImageChoice, CliError> {
    let distribution = catalog.distribution(distribution_id).ok_or_else(|| {
        CliError::creation(
            "Image selection is unavailable",
            format!("distribution {distribution_id} is unavailable for this host"),
            "Choose an image ID shown by the selector as DISTRIBUTION=IMAGE",
        )
    })?;
    if !same_architecture(
        &distribution.architecture,
        &catalog_host_architecture(catalog)?,
    ) {
        return Err(CliError::creation(
            "Image selection is incompatible",
            format!("distribution {distribution_id} is not published for this host"),
            "Choose an image from a host-compatible distribution",
        ));
    }
    let image = distribution
        .images
        .iter()
        .find(|candidate| candidate.id == image_id)
        .ok_or_else(|| {
            CliError::creation(
                "Image selection is unavailable",
                format!("image {image_id} is not published by distribution {distribution_id}"),
                "Choose an image ID shown by the selector as DISTRIBUTION=IMAGE",
            )
        })?
        .clone();
    let kernel = catalog.default_kernel(distribution)?;
    Ok(NewImageChoice {
        distribution: distribution.clone(),
        image: image.clone(),
        kernel,
        expected_bytes: image.size_bytes,
    })
}

fn catalog_host_architecture(
    catalog: &RegistryCatalog,
) -> Result<taumaru_microvm::Architecture, CliError> {
    let _ = catalog;
    host_architecture()
}

pub(crate) fn build_provisioning_plan(
    catalog: &RegistryCatalog,
    choice: NewImageChoice,
) -> Result<NewProvisioningPlan, CliError> {
    let runtime = catalog.select_runtime_packages()?;
    let mut provision_bytes = 0_u64;
    for package in &runtime.packages {
        provision_bytes = checked_size_add(
            provision_bytes,
            binary_package_size(package)?,
            "runtime package",
        )?;
    }
    provision_bytes = checked_size_add(provision_bytes, choice.kernel.size_bytes, "kernel")?;
    provision_bytes =
        checked_size_add(provision_bytes, choice.expected_bytes, "distribution image")?;
    Ok(NewProvisioningPlan {
        runtime_packages: runtime.packages,
        kernel: choice.kernel.clone(),
        provision_bytes,
    })
}

pub(crate) fn build_request(
    choice: &NewImageChoice,
    name: &str,
    disk_size_bytes: u64,
    memory_bytes: u64,
    vcpu_count: u32,
    expose_on_lan: bool,
) -> Result<NewVmRequest, CliError> {
    Ok(NewVmRequest {
        name: name.to_owned(),
        distribution_id: choice.distribution.id.clone(),
        image_id: choice.image.id.clone(),
        disk_size_bytes,
        memory_bytes,
        vcpu_count,
        expose_on_lan,
    })
}

pub(crate) fn creation_sdk_request(request: &NewVmRequest) -> CreateMicroVmRequest {
    CreateMicroVmRequest {
        name: request.name.clone(),
        distribution_id: request.distribution_id.clone(),
        image_id: request.image_id.clone(),
        disk_size_bytes: request.disk_size_bytes,
        vcpu_count: request.vcpu_count,
        memory_bytes: request.memory_bytes,
        expose_on_lan: request.expose_on_lan,
        lan_address: None,
        volume_path: None,
    }
}

#[derive(Clone, Debug)]
struct ImageOption {
    id: String,
    label: String,
}

impl fmt::Display for ImageOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

async fn prompt_name(terminal: TerminalCapabilities) -> Result<String, CliError> {
    let answer = Text::new("Machine name")
        .with_help_message("1-64 ASCII, starts alphanumeric, letters/digits/-/_")
        .with_render_config(prompt_render_config(terminal.color))
        .prompt()
        .map_err(new_prompt_error)?;
    validate_name(answer.trim())
}

async fn prompt_image(
    catalog: &RegistryCatalog,
    sdk: &MicroVmSdk,
    terminal: TerminalCapabilities,
) -> Result<NewImageChoice, CliError> {
    let pairs = catalog.compatible_images();
    if pairs.is_empty() {
        return Err(CliError::creation(
            "No images are available",
            "no host-compatible images are available for this host",
            "Check registry access and host architecture support",
        ));
    }
    let present = sdk.list_present_distribution_images().await?;
    let mut options: Vec<(String, String, ImageOption)> = Vec::with_capacity(pairs.len());
    for (distribution, image) in &pairs {
        let downloaded = present.iter().any(|(ready_distribution, ready_image)| {
            ready_distribution == &distribution.id && ready_image == &image.id
        });
        let marker = if downloaded {
            "downloaded"
        } else {
            "needs download"
        };
        let capabilities_text = if image.capabilities.is_empty() {
            String::new()
        } else {
            format!(" · {}", image.capabilities.join(", "))
        };
        options.push((
            distribution.id.clone(),
            image.id.clone(),
            ImageOption {
                id: format!("{}={}", distribution.id, image.id),
                label: format!(
                    "{} / {} ({} · {}{}) [{}]",
                    distribution.id,
                    image.display_name,
                    image.variant,
                    format_image_bytes(image.size_bytes),
                    capabilities_text,
                    marker
                ),
            },
        ));
    }
    let display: Vec<ImageOption> = options
        .iter()
        .map(|(_, _, option)| option.clone())
        .collect();
    let selected = Select::new("Choose an image", display)
        .with_help_message("↑↓ move  ·  enter confirm")
        .with_page_size(10)
        .with_render_config(prompt_render_config(terminal.color))
        .prompt()
        .map_err(new_prompt_error)?;
    let (distribution_id, image_id) = options
        .into_iter()
        .find(|(_, _, option)| option.id == selected.id)
        .map(|(distribution_id, image_id, _)| (distribution_id, image_id))
        .expect("selected image is listed");
    resolve_choice(catalog, &distribution_id, &image_id)
}

async fn prompt_disk(minimum_bytes: u64, preset: Option<&str>) -> Result<(String, u64), CliError> {
    if let Some(value) = preset {
        let bytes = parse_disk_gb(value)?;
        check_disk_minimum(bytes, minimum_bytes)?;
        return Ok((value.trim().to_owned(), bytes));
    }
    let message = format!("Disk size in GB (minimum {})", format_gb(minimum_bytes));
    let answer = Text::new(&message)
        .with_help_message("positive number in GB, for example 20 or 20.5")
        .with_render_config(prompt_render_config(TerminalCapabilities::detect().color))
        .prompt()
        .map_err(new_prompt_error)?;
    let bytes = parse_disk_gb(answer.trim())?;
    check_disk_minimum(bytes, minimum_bytes)?;
    Ok((answer.trim().to_owned(), bytes))
}

async fn prompt_memory(
    minimum_bytes: u64,
    preset: Option<&str>,
) -> Result<(String, u64), CliError> {
    if let Some(value) = preset {
        let bytes = parse_memory(value)?;
        check_memory_minimum(bytes, minimum_bytes)?;
        return Ok((value.trim().to_owned(), bytes));
    }
    let message = format!("Memory size (minimum {})", format_mb_gb(minimum_bytes));
    let answer = Text::new(&message)
        .with_help_message("xMB or xGB, for example 512MB or 1.5GB")
        .with_render_config(prompt_render_config(TerminalCapabilities::detect().color))
        .prompt()
        .map_err(new_prompt_error)?;
    let bytes = parse_memory(answer.trim())?;
    check_memory_minimum(bytes, minimum_bytes)?;
    Ok((answer.trim().to_owned(), bytes))
}

async fn prompt_vcpus(minimum: u32, preset: Option<&str>) -> Result<u32, CliError> {
    if let Some(value) = preset {
        return parse_vcpus(value);
    }
    let message = format!("vCPU count (minimum {minimum})");
    let answer = Text::new(&message)
        .with_help_message("positive integer count")
        .with_render_config(prompt_render_config(TerminalCapabilities::detect().color))
        .prompt()
        .map_err(new_prompt_error)?;
    parse_vcpus(answer.trim())
}

fn explicit_values_present(arguments: &NewArgs) -> bool {
    arguments.name.is_some()
        || arguments.explicit_name.is_some()
        || arguments.image.is_some()
        || arguments.disk_gb.is_some()
        || arguments.memory.is_some()
        || arguments.vcpus.is_some()
        || arguments.expose_lan
}

pub(crate) async fn run(context: &CliContext, arguments: NewArgs) -> Result<u8, CliError> {
    reject_out_of_scope_flags(&arguments)?;
    let explicit = arguments.non_interactive || explicit_values_present(&arguments);
    if !explicit && !context.terminal.interactive {
        return Err(CliError::creation(
            "An interactive terminal is required",
            "no creation values were supplied and input is not interactive",
            "Pass --non-interactive with --name, --image DISTRIBUTION=IMAGE, --disk-gb, --memory, and --vcpus",
        ));
    }

    if arguments.non_interactive {
        if arguments.name.is_none() && arguments.explicit_name.is_none() {
            return Err(CliError::missing_value(
                "machine name",
                "--name <NAME>",
                "Run microvm new web-01 --non-interactive --image <distribution>=<image> --disk-gb 20 --memory 2GB --vcpus 2",
            ));
        }
        if arguments.image.is_none() {
            return Err(CliError::missing_value(
                "image selection",
                "--image DISTRIBUTION=IMAGE",
                "Run microvm new web-01 --non-interactive --image <distribution>=<image> --disk-gb 20 --memory 2GB --vcpus 2",
            ));
        }
        if arguments.disk_gb.is_none() {
            return Err(CliError::missing_value(
                "disk size",
                "--disk-gb <GB>",
                "Pass --disk-gb 20 with --non-interactive",
            ));
        }
        if arguments.memory.is_none() {
            return Err(CliError::missing_value(
                "memory size",
                "--memory <xMB|xGB>",
                "Pass --memory 2GB with --non-interactive",
            ));
        }
        if arguments.vcpus.is_none() {
            return Err(CliError::missing_value(
                "vCPU count",
                "--vcpus <N>",
                "Pass --vcpus 2 with --non-interactive",
            ));
        }
    }

    let name = if explicit || arguments.name.is_some() || arguments.explicit_name.is_some() {
        Some(resolve_name(
            arguments.name.as_deref(),
            arguments.explicit_name.as_deref(),
        )?)
    } else {
        None
    };

    let client = SdkArtifactClient::new(&context.sdk);
    let catalog_spinner = crate::output::human::CatalogSpinner::new(context.terminal);
    let catalog_result = load_catalog(&client).await;
    catalog_spinner.finish();
    let catalog = catalog_result?;
    let (distribution_count, kernel_count, binary_count) = catalog.counts();
    crate::output::human::write_catalog_ready(
        context.terminal,
        distribution_count,
        kernel_count,
        binary_count,
    )
    .map_err(CliError::from)?;

    let name = match name {
        Some(name) => name,
        None => prompt_name(context.terminal).await?,
    };

    let preset_disk = match arguments.disk_gb.as_deref() {
        Some(value) => Some(parse_disk_gb(value)?),
        None => None,
    };
    let preset_memory = match arguments.memory.as_deref() {
        Some(value) => Some(parse_memory(value)?),
        None => None,
    };
    let preset_vcpus = match arguments.vcpus.as_deref() {
        Some(value) => Some(parse_vcpus(value)?),
        None => None,
    };

    let choice = if let Some(image) = arguments.image.as_deref() {
        if arguments.non_interactive && image.trim().is_empty() {
            return Err(CliError::missing_value(
                "image selection",
                "--image DISTRIBUTION=IMAGE",
                "Run microvm new web-01 --non-interactive --image <distribution>=<image> --disk-gb 20 --memory 2GB --vcpus 2",
            ));
        }
        let (distribution_id, image_id) = parse_single_image(image).map_err(|_| {
            CliError::creation(
                "Image selection is invalid",
                format!("image selection {image:?} must use DISTRIBUTION_ID=IMAGE_ID"),
                "Pass --image DISTRIBUTION=IMAGE exactly once",
            )
        })?;
        resolve_choice(&catalog, &distribution_id, &image_id)?
    } else if explicit && arguments.non_interactive {
        return Err(CliError::missing_value(
            "image selection",
            "--image DISTRIBUTION=IMAGE",
            "Run microvm new web-01 --non-interactive --image <distribution>=<image> --disk-gb 20 --memory 2GB --vcpus 2",
        ));
    } else {
        prompt_image(&catalog, &context.sdk, context.terminal).await?
    };

    let minimum_memory = minimum_memory_bytes(choice.distribution.requirements.min_memory_mb)?;
    let minimum_vcpus = choice.distribution.requirements.min_vcpus;

    if explicit && arguments.non_interactive {
        let disk_text = arguments.disk_gb.as_deref().ok_or_else(|| {
            CliError::missing_value(
                "disk size",
                "--disk-gb <GB>",
                "Pass --disk-gb 20 with --non-interactive",
            )
        })?;
        let memory_text = arguments.memory.as_deref().ok_or_else(|| {
            CliError::missing_value(
                "memory size",
                "--memory <xMB|xGB>",
                "Pass --memory 2GB with --non-interactive",
            )
        })?;
        let vcpu_text = arguments.vcpus.as_deref().ok_or_else(|| {
            CliError::missing_value(
                "vCPU count",
                "--vcpus <N>",
                "Pass --vcpus 2 with --non-interactive",
            )
        })?;
        let disk_size_bytes = preset_disk.expect("disk syntax validated before image resolution");
        check_disk_minimum(disk_size_bytes, choice.image.size_bytes)?;
        let memory_bytes = preset_memory.expect("memory syntax validated before image resolution");
        check_memory_minimum(memory_bytes, minimum_memory)?;
        let vcpu_count = preset_vcpus.expect("vCPU syntax validated before image resolution");
        check_vcpu_minimum(vcpu_count, minimum_vcpus, &choice.distribution.id)?;
        let _ = (disk_text, memory_text, vcpu_text);
        let request = build_request(
            &choice,
            &name,
            disk_size_bytes,
            memory_bytes,
            vcpu_count,
            arguments.expose_lan,
        )?;
        return execute_request(context, &request, &choice).await;
    }

    let (disk_text, disk_size_bytes) =
        prompt_disk(choice.image.size_bytes, arguments.disk_gb.as_deref()).await?;
    let (memory_text, memory_bytes) =
        prompt_memory(minimum_memory, arguments.memory.as_deref()).await?;
    let vcpu_count = prompt_vcpus(minimum_vcpus, arguments.vcpus.as_deref()).await?;
    check_vcpu_minimum(vcpu_count, minimum_vcpus, &choice.distribution.id)?;

    let expose_on_lan = if arguments.expose_lan {
        true
    } else if explicit {
        false
    } else {
        Confirm::new("Expose this VM on the local network?")
            .with_default(false)
            .with_help_message("No keeps the VM host-only  ·  Ctrl-C cancels")
            .with_render_config(prompt_render_config(context.terminal.color))
            .prompt()
            .map_err(new_prompt_error)?
    };

    let request = build_request(
        &choice,
        &name,
        disk_size_bytes,
        memory_bytes,
        vcpu_count,
        expose_on_lan,
    )?;

    if !explicit {
        let plan = build_provisioning_plan(&catalog, choice.clone())?;
        crate::output::human::write_new_review(
            &request,
            &choice,
            &plan,
            &disk_text,
            &memory_text,
            context.terminal,
        )
        .map_err(CliError::from)?;
        let confirmed = Confirm::new("Create this MicroVM?")
            .with_default(false)
            .with_help_message("Enter creates the MicroVM  ·  Ctrl-C cancels")
            .with_render_config(prompt_render_config(context.terminal.color))
            .prompt()
            .map_err(new_prompt_error)?;
        if !confirmed {
            return Err(CliError::cancelled());
        }
    }

    execute_request(context, &request, &choice).await
}

fn reject_out_of_scope_flags(arguments: &NewArgs) -> Result<(), CliError> {
    if let Some(image) = arguments.image.as_deref() {
        let count = image.split(',').count();
        if image.contains(',') && count > 1 {
            return Err(CliError::creation(
                "Image selection is invalid",
                format!("image selection {image:?} must name exactly one image"),
                "Pass --image DISTRIBUTION=IMAGE exactly once",
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn execute_request(
    context: &CliContext,
    request: &NewVmRequest,
    choice: &NewImageChoice,
) -> Result<u8, CliError> {
    let client = SdkArtifactClient::new(&context.sdk);
    let catalog = load_catalog(&client).await?;
    let plan = build_provisioning_plan(&catalog, choice.clone())?;
    let cancellation = DownloadCancellation::new();
    let mut renderer =
        crate::output::human::NewProgressRenderer::new(plan.provision_bytes, context.terminal);
    for package in &plan.runtime_packages {
        let package_id = package.id.clone();
        let package_bytes = binary_package_size(package)?;
        let label = format!("runtime/{package_id}");
        renderer.begin_member(&label, package_bytes);
        let mut forward = super::download::ProgressForwarder::new(&mut renderer, 0, package_bytes);
        let call = client.download_binary(&package_id, &cancellation, |progress| {
            forward.forward(progress);
        });
        let result = call_with_signal(call, &cancellation, tokio::signal::ctrl_c()).await?;
        if let Some(error) = forward.take_error() {
            return Err(error);
        }
        match result {
            OperationResult::Finished(Ok(_)) => {}
            OperationResult::Finished(Err(error)) => {
                return Err(map_provisioning_error(&label, error));
            }
            OperationResult::Cancelled => {
                return Err(CliError::cancelled());
            }
        }
    }

    let kernel_id = plan.kernel.id.clone();
    {
        let label = format!("kernel/{kernel_id}");
        renderer.begin_member(&label, plan.kernel.size_bytes);
        let mut forward =
            super::download::ProgressForwarder::new(&mut renderer, 0, plan.kernel.size_bytes);
        let call = client.download_kernel(&kernel_id, &cancellation, |progress| {
            forward.forward(progress);
        });
        let result = call_with_signal(call, &cancellation, tokio::signal::ctrl_c()).await?;
        if let Some(error) = forward.take_error() {
            return Err(error);
        }
        match result {
            OperationResult::Finished(Ok(_)) => {}
            OperationResult::Finished(Err(error)) => {
                return Err(map_provisioning_error(&label, error));
            }
            OperationResult::Cancelled => {
                return Err(CliError::cancelled());
            }
        }
    }

    {
        let label = format!("image/{}/{}", choice.distribution.id, choice.image.id);
        renderer.begin_member(&label, choice.expected_bytes);
        let mut forward =
            super::download::ProgressForwarder::new(&mut renderer, 0, choice.expected_bytes);
        let call = client.download_distribution_image(
            &choice.distribution.id,
            &choice.image.id,
            &cancellation,
            |progress| forward.forward(progress),
        );
        let result = call_with_signal(call, &cancellation, tokio::signal::ctrl_c()).await?;
        if let Some(error) = forward.take_error() {
            return Err(error);
        }
        match result {
            OperationResult::Finished(Ok(_)) => {}
            OperationResult::Finished(Err(error)) => {
                return Err(map_provisioning_error(&label, error));
            }
            OperationResult::Cancelled => {
                return Err(CliError::cancelled());
            }
        }
    }

    renderer.finish_all(context.terminal);

    let sdk_request = creation_sdk_request(request);
    let interrupted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let interrupted_flag = std::sync::Arc::clone(&interrupted);
    let signal_task = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        interrupted_flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let mut creation_renderer =
        crate::output::human::CreationProgressRenderer::new(context.terminal);
    let creation_result = context
        .sdk
        .create_microvm(
            sdk_request,
            Some(|progress: CreationProgress| {
                creation_renderer.on_creation_progress(progress);
            }),
        )
        .await;
    signal_task.abort();
    creation_renderer.finish();
    let was_interrupted = interrupted.load(std::sync::atomic::Ordering::SeqCst);

    match creation_result {
        Ok(result) => {
            crate::output::human::write_new_result(
                &result,
                request,
                was_interrupted,
                context.terminal,
            )
            .map_err(CliError::from)?;
            if !matches!(result.state, MicroVmState::Configured) {
                return Err(CliError::Sdk(SdkError::Migration(
                    "creation returned without reaching the configured state".to_owned(),
                )));
            }
            Ok(0)
        }
        Err(error) => {
            if matches!(
                error,
                SdkError::ConfigurationConflict { .. } | SdkError::LifecycleConflict { .. }
            ) {
                return Err(CliError::conflict(
                    "MicroVM name is already in use",
                    error.to_string(),
                    "Choose another --name or remove the existing VM",
                ));
            }
            Err(map_creation_error(error, was_interrupted))
        }
    }
}

fn map_provisioning_error(label: &str, error: SdkError) -> CliError {
    CliError::provisioning(
        "MicroVM prerequisites could not be prepared",
        format!("{label} failed: {error}"),
        "Check registry access and local storage, then retry; verified prerequisites will be reused",
    )
}

fn map_creation_error(error: SdkError, was_interrupted: bool) -> CliError {
    let suffix = if was_interrupted {
        " The interrupt was reported after the operation settled."
    } else {
        ""
    };
    CliError::provisioning(
        "MicroVM creation failed",
        format!("{error}{suffix}"),
        "Check the reported cause, then retry; verified prerequisites will be reused",
    )
}

#[cfg(test)]
mod tests {
    use super::{
        check_disk_minimum, check_memory_minimum, check_vcpu_minimum, format_gb, format_mb_gb,
        parse_disk_gb, parse_memory, parse_single_image, parse_vcpus, resolve_name, validate_name,
    };

    #[test]
    fn name_rules_reject_invalid_values() {
        assert_eq!(validate_name("web-01").expect("valid name"), "web-01");
        assert!(validate_name("").is_err());
        assert!(validate_name("-lead").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name("has/slash").is_err());
        assert!(validate_name(&"x".repeat(65)).is_err());
    }

    #[test]
    fn positional_and_flag_names_must_agree() {
        assert_eq!(
            resolve_name(Some("web-01"), Some("web-01")).expect("agreeing names"),
            "web-01"
        );
        assert!(resolve_name(Some("web-01"), Some("db-01")).is_err());
        assert!(resolve_name(None, None).is_err());
    }

    #[test]
    fn disk_values_accept_optional_gb_suffix() {
        let gib = 1024 * 1024 * 1024;
        assert_eq!(parse_disk_gb("10GB").expect("upper"), 10 * gib);
        assert_eq!(parse_disk_gb("10Gb").expect("mixed"), 10 * gib);
        assert_eq!(parse_disk_gb("10gb").expect("lower"), 10 * gib);
        assert_eq!(parse_disk_gb("10 GB").expect("spaced"), 10 * gib);
        assert!(parse_disk_gb("10MB").is_err());
    }

    #[test]
    fn disk_values_convert_with_ceiling() {
        assert_eq!(parse_disk_gb("20").expect("disk"), 20 * 1024 * 1024 * 1024);
        let fractional = parse_disk_gb("20.5").expect("fractional disk");
        assert!(fractional > 20 * 1024 * 1024 * 1024);
        assert!(parse_disk_gb("0").is_err());
        assert!(parse_disk_gb("-2").is_err());
        assert!(parse_disk_gb("lots").is_err());
    }

    #[test]
    fn disk_minimum_is_shown_in_gb() {
        let error = check_disk_minimum(1, 20 * 1024 * 1024 * 1024).expect_err("too small");
        assert!(error.user_message(false).contains("20 GB"));
    }

    #[test]
    fn memory_values_accept_mb_gb_forms() {
        assert_eq!(parse_memory("512MB").expect("mb"), 512 * 1024 * 1024);
        assert_eq!(
            parse_memory("512 MB").expect("spaced mb"),
            512 * 1024 * 1024
        );
        assert_eq!(parse_memory("2GB").expect("gb"), 2 * 1024 * 1024 * 1024);
        assert_eq!(
            parse_memory("2 gb").expect("lower spaced"),
            2 * 1024 * 1024 * 1024
        );
        let fractional = parse_memory("1.5GB").expect("fractional memory");
        assert_eq!(
            fractional,
            (1.5_f64 * 1024.0 * 1024.0 * 1024.0).ceil() as u64
        );
        assert_eq!(
            parse_memory("512").expect("bare means MB"),
            512 * 1024 * 1024
        );
        assert_eq!(
            parse_memory(" 512 ").expect("trimmed bare"),
            512 * 1024 * 1024
        );
        assert!(parse_memory("2TB").is_err());
        assert!(parse_memory("0MB").is_err());
    }

    #[test]
    fn memory_minimum_is_shown_in_input_units() {
        assert_eq!(format_mb_gb(512 * 1024 * 1024), "512 MB");
        assert_eq!(format_gb(20 * 1024 * 1024 * 1024), "20 GB");
        let error = check_memory_minimum(1, 512 * 1024 * 1024).expect_err("too small");
        assert!(error.user_message(false).contains("512 MB"));
    }

    #[test]
    fn vcpu_values_require_positive_integers() {
        assert_eq!(parse_vcpus("2").expect("vcpus"), 2);
        assert!(parse_vcpus("0").is_err());
        assert!(parse_vcpus("1.5").is_err());
        assert!(check_vcpu_minimum(1, 2, "distro").is_err());
        assert!(check_vcpu_minimum(2, 2, "distro").is_ok());
    }

    #[test]
    fn provisioning_plan_orders_runtime_kernel_image_with_checked_totals() {
        use super::{build_provisioning_plan, resolve_choice};
        use taumaru_microvm::{
            Architecture, BinaryFile, BinaryPackage, BootConfiguration, Distribution,
            DistributionImage, DistributionRequirements, FilesystemMetadata, Kernel,
        };

        fn kernel(id: &str) -> Kernel {
            Kernel {
                id: id.to_owned(),
                name: id.to_owned(),
                display_name: id.to_owned(),
                version: "6.2.0".to_owned(),
                architecture: Architecture::X86_64,
                path: format!("kernels/{id}/vmlinux"),
                url: format!("https://example.invalid/{id}"),
                filename: "vmlinux".to_owned(),
                size_bytes: 13,
                sha256: "0".repeat(64),
                format: "elf".to_owned(),
                mime_type: "application/octet-stream".to_owned(),
                elf: None,
                modified_at: "2026-01-01T00:00:00Z".to_owned(),
            }
        }

        fn image(id: &str) -> DistributionImage {
            DistributionImage {
                id: id.to_owned(),
                name: id.to_owned(),
                display_name: id.to_owned(),
                description: id.to_owned(),
                variant: "minimal".to_owned(),
                path: format!("images/{id}"),
                url: format!("https://example.invalid/{id}"),
                filename: format!("{id}.ext4"),
                format: "ext4".to_owned(),
                filesystem: FilesystemMetadata {
                    filesystem_type: "ext4".to_owned(),
                    uuid: format!("uuid-{id}"),
                    block_size: 4096,
                    block_count: 1,
                    free_blocks: 0,
                    inode_count: 1,
                    free_inodes: 0,
                    features: Vec::new(),
                },
                size_bytes: 37,
                sha256: "0".repeat(64),
                capabilities: Vec::new(),
                mime_type: "application/octet-stream".to_owned(),
                modified_at: "2026-01-01T00:00:00Z".to_owned(),
            }
        }

        fn binary_file(name: &str, size_bytes: u64) -> BinaryFile {
            BinaryFile {
                name: name.to_owned(),
                path: format!("binaries/runtime/{name}"),
                url: format!("https://example.invalid/{name}"),
                filename: name.to_owned(),
                size_bytes,
                sha256: "0".repeat(64),
                mime_type: "application/octet-stream".to_owned(),
                executable: true,
                mode: Some("755".to_owned()),
                permissions: None,
                format: Some("elf".to_owned()),
                elf: None,
                modified_at: "2026-01-01T00:00:00Z".to_owned(),
            }
        }

        let catalog = crate::commands::download::RegistryCatalog::new(
            vec![kernel("kernel-a")],
            vec![BinaryPackage {
                id: "runtime-new".to_owned(),
                name: "firecracker".to_owned(),
                display_name: "runtime-new".to_owned(),
                description: None,
                version: "2.0.0".to_owned(),
                architecture: Architecture::X86_64,
                files: vec![binary_file("firecracker", 19), binary_file("firectl", 23)],
            }],
            vec![Distribution {
                id: "distro-a".to_owned(),
                name: "distro-a".to_owned(),
                display_name: "distro-a".to_owned(),
                description: "distro-a".to_owned(),
                distribution: "distro-a".to_owned(),
                version: "1.0".to_owned(),
                codename: "test".to_owned(),
                architecture: Architecture::X86_64,
                vendor: "Taumaru".to_owned(),
                homepage: "https://example.invalid".to_owned(),
                default_kernel: "kernel-a".to_owned(),
                supported_kernels: vec!["kernel-a".to_owned()],
                boot: BootConfiguration {
                    root_device: "/dev/vda".to_owned(),
                    kernel_args: Vec::new(),
                },
                requirements: DistributionRequirements {
                    min_memory_mb: 128,
                    min_vcpus: 1,
                },
                images: vec![image("image-a")],
            }],
            Architecture::X86_64,
        )
        .expect("test catalog should be valid");

        let choice = resolve_choice(&catalog, "distro-a", "image-a").expect("choice");
        let plan = build_provisioning_plan(&catalog, choice).expect("plan");
        assert_eq!(
            plan.runtime_packages
                .iter()
                .map(|package| package.id.clone())
                .collect::<Vec<_>>(),
            vec!["runtime-new".to_owned()]
        );
        assert_eq!(plan.kernel.id, "kernel-a");
        assert_eq!(plan.provision_bytes, 19 + 23 + 13 + 37);
    }

    #[test]
    fn creation_conflicts_and_interruptions_report_actionable_outcomes() {
        use super::{map_creation_error, map_provisioning_error};
        use taumaru_microvm::SdkError;

        let conflict = SdkError::ConfigurationConflict {
            name: "web-01".to_owned(),
            field: "memory_bytes".to_owned(),
            existing: "1".to_owned(),
            requested: "2".to_owned(),
        };
        assert!(
            map_creation_error(conflict, false)
                .user_message(false)
                .contains("verified prerequisites will be reused")
        );
        let interrupted = map_creation_error(
            SdkError::Migration("settled without success".to_owned()),
            true,
        );
        assert!(
            interrupted
                .user_message(false)
                .contains("after the operation settled")
        );
        let provisioned = map_provisioning_error(
            "kernel/kernel-a",
            SdkError::Migration("registry is down".to_owned()),
        );
        let message = provisioned.user_message(false).to_ascii_lowercase();
        assert!(message.contains("kernel/kernel-a"));
        assert!(message.contains("verified prerequisites will be reused"));
    }

    #[test]
    fn prompt_cancellation_reports_creation_not_download() {
        use super::new_prompt_error;

        let cancelled = new_prompt_error(inquire::InquireError::OperationCanceled);
        assert_eq!(cancelled.exit_code(), 130);
        let message = cancelled.user_message(false);
        assert!(message.contains("MicroVM creation cancelled"));
        assert!(!message.contains("Download"));
        assert!(!message.contains("artifacts download"));
    }

    #[test]
    fn single_image_form_is_scoped() {
        let (distribution, image) =
            parse_single_image("ubuntu-24.04=ubuntu-24.04-docker").expect("scoped image");
        assert_eq!(distribution, "ubuntu-24.04");
        assert_eq!(image, "ubuntu-24.04-docker");
        assert!(parse_single_image("bare-image").is_err());
    }
}
