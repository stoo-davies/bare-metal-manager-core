//! Stages the host's `lldpcli` runtime for containerized DPU-agent operation.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use eyre::{Context, ContextCompat, bail};
use nix::unistd::{Gid, Uid, setgid, setgroups, setuid};

const HOST_LLDPCLI: &str = "/host/usr/sbin/lldpcli";
const HOST_LIB: &str = "/host/lib";
const HOST_LOADER: &str = "/host/lib/ld-linux-aarch64.so.1";
const HOST_LIBRARY_PATH: &str = "/host/lib/aarch64-linux-gnu";

const RUNTIME_ROOT: &str = "/data/host-lldp";
const RUNTIME_CLIENT: &str = "/data/host-lldp/libexec/lldpcli";
const RUNTIME_LIB: &str = "/data/host-lldp/lib";
const RUNTIME_LOADER: &str = "/data/host-lldp/lib/ld-linux-aarch64.so.1";
const SOCKET_PATH: &str = "/run/lldpd.socket";

/// Copy the host-matched LLDP client and its library closure into `/data`.
pub fn stage() -> eyre::Result<()> {
    stage_at(
        Path::new(HOST_LLDPCLI),
        Path::new(HOST_LIB),
        Path::new(HOST_LOADER),
        Path::new(HOST_LIBRARY_PATH),
        Path::new(RUNTIME_ROOT),
    )
}

fn stage_at(
    host_client: &Path,
    host_lib: &Path,
    host_loader: &Path,
    host_library_path: &Path,
    runtime_root: &Path,
) -> eyre::Result<()> {
    let output = Command::new(host_loader)
        .arg("--list")
        .arg("--library-path")
        .arg(host_library_path)
        .arg(host_client)
        .output()
        .wrap_err_with(|| format!("failed to inspect {}", host_client.display()))?;
    if !output.status.success() {
        bail!(
            "host loader failed to inspect {}: {}",
            host_client.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let dependencies = parse_dependencies(
        &String::from_utf8_lossy(&output.stdout),
        host_lib,
        host_loader,
    )?;
    let parent = runtime_root
        .parent()
        .wrap_err("LLDP runtime path has no parent")?;
    let temporary = tempfile::Builder::new()
        .prefix(".host-lldp.")
        .tempdir_in(parent)
        .wrap_err_with(|| format!("failed to create LLDP runtime below {}", parent.display()))?;
    let temporary_root = temporary.path();
    let temporary_lib = temporary_root.join("lib");
    let temporary_client = temporary_root.join("libexec/lldpcli");
    let temporary_loader = temporary_lib.join("ld-linux-aarch64.so.1");
    fs::create_dir_all(&temporary_lib)?;
    fs::create_dir_all(
        temporary_client
            .parent()
            .wrap_err("staged LLDP client path has no parent")?,
    )?;
    for directory in [
        temporary_root,
        temporary_lib.as_path(),
        temporary_client
            .parent()
            .wrap_err("staged LLDP client path has no parent")?,
    ] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o755))?;
    }

    copy_file(host_client, &temporary_client, 0o755)?;
    copy_file(host_loader, &temporary_loader, 0o755)?;
    for (soname, source) in dependencies {
        copy_file(&source, &temporary_lib.join(soname), 0o644)?;
    }

    let closure = Command::new(&temporary_loader)
        .arg("--list")
        .arg("--inhibit-cache")
        .arg("--library-path")
        .arg(&temporary_lib)
        .arg(&temporary_client)
        .env_remove("LD_AUDIT")
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_PRELOAD")
        .output()
        .wrap_err("failed to inspect staged lldpcli dependencies")?;
    if !closure.status.success() {
        bail!(
            "staged lldpcli dependency inspection failed: {}",
            String::from_utf8_lossy(&closure.stderr).trim()
        );
    }
    validate_staged_dependencies(
        &String::from_utf8_lossy(&closure.stdout),
        &temporary_lib,
        &temporary_loader,
    )?;

    let version = Command::new(&temporary_loader)
        .arg("--inhibit-cache")
        .arg("--library-path")
        .arg(&temporary_lib)
        .arg(&temporary_client)
        .arg("-vv")
        .env_remove("LD_AUDIT")
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_PRELOAD")
        .output()
        .wrap_err("failed to execute staged lldpcli")?;
    if !version.status.success() {
        bail!(
            "staged lldpcli validation failed: {}",
            String::from_utf8_lossy(&version.stderr).trim()
        );
    }
    fs::write(
        temporary_root.join("version.txt"),
        if version.stdout.is_empty() {
            &version.stderr
        } else {
            &version.stdout
        },
    )?;

    if runtime_root.exists() {
        fs::remove_dir_all(runtime_root).wrap_err_with(|| {
            format!(
                "failed to replace LLDP runtime at {}",
                runtime_root.display()
            )
        })?;
    }
    let temporary_root = temporary.keep();
    fs::rename(&temporary_root, runtime_root).wrap_err_with(|| {
        format!(
            "failed to install LLDP runtime from {} at {}",
            temporary_root.display(),
            runtime_root.display()
        )
    })?;
    Ok(())
}

fn copy_file(source: &Path, destination: &Path, mode: u32) -> eyre::Result<()> {
    fs::copy(source, destination).wrap_err_with(|| {
        format!(
            "failed to copy {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    fs::set_permissions(destination, fs::Permissions::from_mode(mode))?;
    Ok(())
}

fn validate_staged_dependencies(
    output: &str,
    staged_lib: &Path,
    staged_loader: &Path,
) -> eyre::Result<()> {
    let canonical_staged_lib = fs::canonicalize(staged_lib)
        .wrap_err_with(|| format!("failed to resolve {}", staged_lib.display()))?;
    let canonical_staged_loader = fs::canonicalize(staged_loader)
        .wrap_err_with(|| format!("failed to resolve {}", staged_loader.display()))?;
    let mut dependency_count = 0;
    let mut loader_seen = false;

    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let (name, resolved) = if let Some((name, resolved)) = line.split_once("=>") {
            let name = name.trim();
            let resolved = resolved.trim();
            if resolved.starts_with("not found") {
                bail!("staged lldpcli dependency {name} was not found");
            }
            let resolved = resolved
                .split_whitespace()
                .next()
                .wrap_err_with(|| format!("missing path for staged lldpcli dependency {name}"))?;
            (name, Path::new(resolved))
        } else {
            let entry = line
                .split_whitespace()
                .next()
                .wrap_err("missing staged lldpcli dependency entry")?;
            if entry == "linux-vdso.so.1" {
                continue;
            }
            let resolved = Path::new(entry);
            if !resolved.is_absolute() {
                bail!("invalid staged lldpcli dependency entry {line:?}");
            }
            (entry, resolved)
        };

        let canonical_resolved = fs::canonicalize(resolved).wrap_err_with(|| {
            format!(
                "failed to resolve staged lldpcli dependency {name} at {}",
                resolved.display()
            )
        })?;
        if !canonical_resolved.starts_with(&canonical_staged_lib) {
            bail!(
                "staged lldpcli dependency {name} resolved outside {}: {}",
                staged_lib.display(),
                canonical_resolved.display()
            );
        }
        if Path::new(name).is_absolute() {
            if canonical_resolved != canonical_staged_loader {
                bail!(
                    "staged lldpcli loader resolved to unexpected path {}",
                    canonical_resolved.display()
                );
            }
            loader_seen = true;
        } else {
            dependency_count += 1;
        }
    }

    if dependency_count == 0 {
        bail!("staged loader reported no lldpcli dependencies");
    }
    if !loader_seen {
        bail!("staged loader did not report itself in the lldpcli dependency closure");
    }
    Ok(())
}

fn parse_dependencies(
    output: &str,
    host_lib: &Path,
    host_loader: &Path,
) -> eyre::Result<BTreeMap<String, PathBuf>> {
    let canonical_host_lib = fs::canonicalize(host_lib)
        .wrap_err_with(|| format!("failed to resolve {}", host_lib.display()))?;
    let canonical_host_loader = fs::canonicalize(host_loader)
        .wrap_err_with(|| format!("failed to resolve {}", host_loader.display()))?;
    let mut dependencies = BTreeMap::new();

    for line in output.lines() {
        let Some((name, resolved)) = line.split_once("=>") else {
            continue;
        };
        let name = name.trim();
        let resolved = resolved.trim();
        if resolved.starts_with("not found") {
            bail!("host lldpcli dependency {name} was not found");
        }
        let source = resolved
            .split_whitespace()
            .next()
            .wrap_err_with(|| format!("missing path for host lldpcli dependency {name}"))?;
        let source = Path::new(source);
        let mounted_source = if source.starts_with(host_lib) {
            source.to_path_buf()
        } else if let Ok(relative) = source.strip_prefix("/lib") {
            host_lib.join(relative)
        } else {
            bail!(
                "host lldpcli dependency {name} resolved outside /lib: {}",
                source.display()
            );
        };
        let canonical_source = fs::canonicalize(&mounted_source).wrap_err_with(|| {
            format!(
                "failed to resolve host lldpcli dependency {name} at {}",
                mounted_source.display()
            )
        })?;
        if !canonical_source.starts_with(&canonical_host_lib) {
            bail!(
                "host lldpcli dependency {name} escapes {}",
                host_lib.display()
            );
        }

        let name_path = Path::new(name);
        if name_path.is_absolute() {
            if name_path.file_name() == host_loader.file_name()
                && canonical_source == canonical_host_loader
            {
                // The loader reports the target's ELF interpreter as a mapping
                // too. It is copied separately under the fixed runtime name.
                continue;
            }
            bail!("invalid host lldpcli dependency name {name:?}");
        }
        if name.is_empty() || name_path.file_name().and_then(|part| part.to_str()) != Some(name) {
            bail!("invalid host lldpcli dependency name {name:?}");
        }
        dependencies.insert(name.to_string(), canonical_source);
    }

    if dependencies.is_empty() {
        bail!("host loader reported no lldpcli dependencies");
    }
    Ok(dependencies)
}

/// Execute the staged host client as the owner of the mounted control socket.
pub fn exec(args: impl IntoIterator<Item = OsString>) -> eyre::Result<()> {
    let socket =
        fs::metadata(SOCKET_PATH).wrap_err_with(|| format!("failed to inspect {SOCKET_PATH}"))?;
    let uid = Uid::from_raw(socket.uid());
    let gid = Gid::from_raw(socket.gid());

    setgroups(&[gid]).wrap_err("failed to set lldpcli supplementary group")?;
    setgid(gid).wrap_err("failed to set lldpcli group")?;
    setuid(uid).wrap_err("failed to set lldpcli user")?;

    let error = Command::new(RUNTIME_LOADER)
        .arg("--inhibit-cache")
        .arg("--library-path")
        .arg(RUNTIME_LIB)
        .arg(RUNTIME_CLIENT)
        .args(args)
        .env_remove("LD_AUDIT")
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_PRELOAD")
        .exec();
    Err(error).wrap_err("failed to execute staged host lldpcli")
}

#[cfg(test)]
mod tests {
    use super::*;
    use carbide_test_support::Outcome::{Fails, Yields};
    use carbide_test_support::scenarios;
    use carbide_test_support::{Case, check_cases};

    #[test]
    fn parses_loader_dependencies() {
        let fixture = tempfile::tempdir().unwrap();
        let host_lib = fixture.path().join("lib");
        let architecture_lib = host_lib.join("aarch64-linux-gnu");
        let host_loader = host_lib.join("ld-linux-aarch64.so.1");
        fs::create_dir_all(&architecture_lib).unwrap();
        fs::write(&host_loader, "loader").unwrap();
        for library in ["liblldpctl.so.4", "libc.so.6"] {
            fs::write(architecture_lib.join(library), library).unwrap();
        }

        scenarios!(run = |output: &str| parse_dependencies(output, &host_lib, &host_loader)
            .map(|dependencies| dependencies.keys().cloned().collect::<Vec<_>>())
            .map_err(drop);
            "valid loader output" {
                "liblldpctl.so.4 => /lib/aarch64-linux-gnu/liblldpctl.so.4 (0x1)\n/lib/ld-linux-aarch64.so.1 => /lib/ld-linux-aarch64.so.1 (0x2)\nlibc.so.6 => /lib/aarch64-linux-gnu/libc.so.6 (0x3)" =>
                    Yields(vec!["libc.so.6".to_string(), "liblldpctl.so.4".to_string()]),
            }
            "invalid loader output" {
                "liblldpctl.so.4 => not found" => Fails,
                "liblldpctl.so.4 => /usr/lib/liblldpctl.so.4 (0x1)" => Fails,
                "/usr/lib/liblldpctl.so.4 => /lib/aarch64-linux-gnu/liblldpctl.so.4 (0x1)" => Fails,
                "linux-vdso.so.1 (0x1)" => Fails,
            }
        );
    }

    #[test]
    fn validates_staged_dependency_paths() {
        let fixture = tempfile::tempdir().unwrap();
        let staged_lib = fixture.path().join("staged/lib");
        let staged_loader = staged_lib.join("ld-linux-aarch64.so.1");
        let staged_dependency = staged_lib.join("liblldpctl.so.4");
        let outside_dependency = fixture.path().join("libc.so.6");
        fs::create_dir_all(&staged_lib).unwrap();
        fs::write(&staged_loader, "loader").unwrap();
        fs::write(&staged_dependency, "lldp").unwrap();
        fs::write(&outside_dependency, "libc").unwrap();

        check_cases(
            [
                Case {
                    scenario: "complete closure resolves from staged directory",
                    input: format!(
                        "linux-vdso.so.1 (0x1)\nliblldpctl.so.4 => {} (0x2)\n{} (0x3)",
                        staged_dependency.display(),
                        staged_loader.display()
                    ),
                    expect: Yields(()),
                },
                Case {
                    scenario: "missing dependency is rejected",
                    input: "liblldpctl.so.4 => not found".to_string(),
                    expect: Fails,
                },
                Case {
                    scenario: "system fallback is rejected",
                    input: format!("libc.so.6 => {} (0x1)", outside_dependency.display()),
                    expect: Fails,
                },
                Case {
                    scenario: "unexpected absolute entry is rejected",
                    input: format!("{} (0x1)", outside_dependency.display()),
                    expect: Fails,
                },
                Case {
                    scenario: "malformed output is rejected",
                    input: "liblldpctl.so.4".to_string(),
                    expect: Fails,
                },
                Case {
                    scenario: "empty dependency closure is rejected",
                    input: "linux-vdso.so.1 (0x1)".to_string(),
                    expect: Fails,
                },
                Case {
                    scenario: "dependency closure without loader is rejected",
                    input: format!("liblldpctl.so.4 => {} (0x1)", staged_dependency.display()),
                    expect: Fails,
                },
            ],
            |output| {
                validate_staged_dependencies(&output, &staged_lib, &staged_loader).map_err(drop)
            },
        );
    }

    #[test]
    fn stages_runtime_with_host_loader() {
        let fixture = tempfile::tempdir().unwrap();
        let host_client = fixture.path().join("lldpcli");
        let host_lib = fixture.path().join("lib");
        let architecture_lib = host_lib.join("aarch64-linux-gnu");
        let host_loader = host_lib.join("ld-linux-aarch64.so.1");
        let dependency = architecture_lib.join("liblldpctl.so.4");
        let runtime_root = fixture.path().join("data/host-lldp");
        fs::create_dir_all(&architecture_lib).unwrap();
        fs::create_dir_all(runtime_root.parent().unwrap()).unwrap();
        fs::write(&host_client, "client").unwrap();
        fs::write(&dependency, "dependency").unwrap();
        fs::write(
            &host_loader,
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"--list\" ]; then\n  if [ \"$0\" = \"{}\" ]; then\n    dependency=\"{}\"\n  else\n    dependency=\"$(dirname \"$0\")/liblldpctl.so.4\"\n  fi\n  echo \"liblldpctl.so.4 => $dependency (0x1)\"\n  echo \"$0 (0x2)\"\nelse\n  echo 'lldpcli 1.0.18'\nfi\n",
                host_loader.display(),
                dependency.display(),
            ),
        )
        .unwrap();
        fs::set_permissions(&host_loader, fs::Permissions::from_mode(0o755)).unwrap();

        stage_at(
            &host_client,
            &host_lib,
            &host_loader,
            &architecture_lib,
            &runtime_root,
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(runtime_root.join("lib/liblldpctl.so.4")).unwrap(),
            "dependency"
        );
        assert_eq!(
            fs::read_to_string(runtime_root.join("libexec/lldpcli")).unwrap(),
            "client"
        );
        assert_eq!(
            fs::metadata(&runtime_root).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert!(runtime_root.join("version.txt").is_file());
    }

    #[test]
    fn does_not_publish_runtime_with_external_dependency() {
        let fixture = tempfile::tempdir().unwrap();
        let host_client = fixture.path().join("lldpcli");
        let host_lib = fixture.path().join("lib");
        let architecture_lib = host_lib.join("aarch64-linux-gnu");
        let host_loader = host_lib.join("ld-linux-aarch64.so.1");
        let dependency = architecture_lib.join("liblldpctl.so.4");
        let outside_dependency = fixture.path().join("outside.so");
        let runtime_root = fixture.path().join("data/host-lldp");
        fs::create_dir_all(&architecture_lib).unwrap();
        fs::create_dir_all(runtime_root.parent().unwrap()).unwrap();
        fs::write(&host_client, "client").unwrap();
        fs::write(&dependency, "dependency").unwrap();
        fs::write(&outside_dependency, "outside").unwrap();
        fs::write(
            &host_loader,
            format!(
                "#!/bin/sh\nif [ \"$0\" = \"{}\" ]; then\n  dependency=\"{}\"\nelse\n  dependency=\"{}\"\nfi\necho \"liblldpctl.so.4 => $dependency (0x1)\"\necho \"$0 (0x2)\"\n",
                host_loader.display(),
                dependency.display(),
                outside_dependency.display(),
            ),
        )
        .unwrap();
        fs::set_permissions(&host_loader, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            stage_at(
                &host_client,
                &host_lib,
                &host_loader,
                &architecture_lib,
                &runtime_root,
            )
            .is_err()
        );
        assert!(!runtime_root.exists());
    }
}
