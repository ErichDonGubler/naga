use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{ExitCode, Stdio},
};

use anyhow::{bail, ensure, Context};
use cli::Args;

use crate::{
    cli::{Subcommand, ValidateHlslCommand, ValidateSubcommand},
    fs::{open_file, remove_dir_all, remove_file},
    glob::visit_files,
    path::join_path,
    process::{which, EasyCommand},
    result::LogIfError,
};

mod cli;
mod fs;
mod glob;
mod path;
mod process;
mod result;

fn main() -> ExitCode {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .format_indent(Some(0))
        .init();

    let args = Args::parse();

    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("{e:?}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> anyhow::Result<()> {
    let snapshots_base_out = join_path(["tests", "out"]);

    let Args { subcommand } = args;

    assert!(which("cargo").is_ok());

    match subcommand {
        Subcommand::All => {
            EasyCommand::simple("cargo", ["fmt"]).success()?;
            EasyCommand::simple("cargo", ["test", "--all-features", "--workspace"]).success()?;
            EasyCommand::simple(
                "cargo",
                [
                    "clippy",
                    "--all-features",
                    "--workspace",
                    "--",
                    "-D",
                    "warnings",
                ],
            )
            .success()?;
            Ok(())
        }
        Subcommand::Clean => {
            let mut found_err = false;
            visit_files(
                ".",
                "*.{metal,air,metallib,vert,frag,comp,spv}",
                &mut found_err,
                |file, _| {
                    remove_file(file).with_context(|| format!("failed to remove file {file:?}"))
                },
            );
            ensure!(
                !found_err,
                "failed to clean one or more files, see above output for more details"
            );
            Ok(())
        }
        Subcommand::Bench { clean } => {
            if clean {
                let criterion_artifact_dir = join_path(["target", "criterion"]);
                log::info!("removing {}", criterion_artifact_dir.display());
                remove_dir_all(&criterion_artifact_dir)
                    .with_context(|| format!("failed to remove {criterion_artifact_dir:?}"))?;
            }
            EasyCommand::simple("cargo", ["bench"]).success()
        }
        Subcommand::Validate(cmd) => {
            let mut found_err = false;
            let ack_visiting = |path: &Path| log::info!("Validating {}", path.display());
            match cmd {
                ValidateSubcommand::Spirv => {
                    visit_files(
                        snapshots_base_out,
                        "spv/*.spvasm",
                        &mut found_err,
                        |path, _| {
                            ack_visiting(path);
                            let second_line = {
                                let mut file = BufReader::new(open_file(path)?);
                                let mut buf = String::new();
                                file.read_line(&mut buf).with_context(|| {
                                    format!("failed to read first line from {path:?}")
                                })?;
                                buf.clear();
                                file.read_line(&mut buf).with_context(|| {
                                    format!("failed to read second line from {path:?}")
                                })?;
                                buf
                            };
                            let expected_header_prefix = "; Version: ";
                            let Some(version) =
                                second_line.strip_prefix(expected_header_prefix) else {
                                    bail!(
                                        "no {expected_header_prefix:?} header found in {path:?}"
                                    );
                                };
                            let file = open_file(path)?;
                            let mut spirv_as_cmd = EasyCommand::new("spirv-as", |cmd| {
                                cmd.stdin(Stdio::from(file))
                                    .stdout(Stdio::piped())
                                    .arg("--target-env")
                                    .arg(format!("spv{version}"))
                                    .args(["-o", "-"])
                            });
                            let child = spirv_as_cmd
                                .spawn()
                                .with_context(|| format!("failed to spawn {cmd:?}"))?;
                            EasyCommand::new("spirv-val", |cmd| cmd.stdin(child.stdout.unwrap()))
                                .success()
                        },
                    );
                }
                ValidateSubcommand::Metal => {
                    let xcrun = "xcrun";
                    which(xcrun)?;
                    visit_files(
                        snapshots_base_out,
                        "msl/*.msl",
                        &mut found_err,
                        |path, _| {
                            ack_visiting(path);
                            let first_line = {
                                let mut file = BufReader::new(open_file(path)?);
                                let mut buf = String::new();
                                file.read_line(&mut buf).with_context(|| {
                                    format!("failed to read header from {path:?}")
                                })?;
                                buf
                            };
                            let expected_header_prefix = "// language: ";
                            let Some(language) =
                                first_line.strip_prefix(expected_header_prefix) else {
                                    bail!(
                                        "no {expected_header_prefix:?} header found in {path:?}"
                                    );
                                };
                            let language = language.strip_suffix('\n').unwrap_or(language);

                            let file = open_file(path)?;
                            EasyCommand::new(xcrun, |cmd| {
                                cmd.stdin(Stdio::from(file))
                                    .args(["-sdk", "macosx", "metal", "-mmacosx-version-min=10.11"])
                                    .arg(format!("-std=macos-{language}"))
                                    .args(["-x", "metal", "-", "-o", "/dev/null"])
                            })
                            .success()
                        },
                    )
                }
                ValidateSubcommand::Glsl => {
                    let glslang_validator = "glslangValidator";
                    which(glslang_validator)?;
                    for (glob, type_arg) in [
                        ("glsl/*.Vertex.glsl", "vert"),
                        ("glsl/*.Fragment.glsl", "frag"),
                        ("glsl/*.Compute.glsl", "comp"),
                    ] {
                        visit_files(&snapshots_base_out, glob, &mut found_err, |path, _| {
                            ack_visiting(path);
                            let file = open_file(path)?;
                            EasyCommand::new(glslang_validator, |cmd| {
                                cmd.stdin(Stdio::from(file))
                                    .args(["--stdin", "-S"])
                                    .arg(type_arg)
                            })
                            .success()
                        });
                    }
                }
                ValidateSubcommand::Dot => {
                    let dot = "dot";
                    which(dot)?;
                    visit_files(
                        snapshots_base_out,
                        "dot/*.dot",
                        &mut found_err,
                        |path, _| {
                            ack_visiting(path);
                            let file = open_file(path)?;
                            EasyCommand::new(dot, |cmd| {
                                cmd.stdin(Stdio::from(file)).stdout(Stdio::null())
                            })
                            .success()
                        },
                    )
                }
                ValidateSubcommand::Wgsl => visit_files(
                    snapshots_base_out,
                    "wgsl/*.wgsl",
                    &mut found_err,
                    |path, _| {
                        ack_visiting(path);
                        EasyCommand::new("cargo", |cmd| cmd.args(["run", "--"]).arg(path)).success()
                    },
                ),
                ValidateSubcommand::Hlsl(cmd) => {
                    let visit_hlsl = |consume_config_item: &mut dyn FnMut(
                        &Path,
                        hlsl_snapshots::ConfigItem,
                    )
                        -> anyhow::Result<()>| {
                        visit_files(
                            snapshots_base_out,
                            "hlsl/*.hlsl",
                            &mut found_err,
                            |path, found_err| {
                                ack_visiting(path);
                                let hlsl_snapshots::Config {
                                    vertex,
                                    fragment,
                                    compute,
                                } = hlsl_snapshots::Config::from_path(path.with_extension("ron"))?;
                                [vertex, fragment, compute].into_iter().flatten().for_each(
                                    |shader| {
                                        consume_config_item(path, shader).log_if_err(found_err);
                                    },
                                );
                                Ok(())
                            },
                        )
                    };
                    let validate = |bin, file: &_, config_item, params: &[_]| {
                        let hlsl_snapshots::ConfigItem {
                            entry_point,
                            target_profile,
                        } = config_item;
                        EasyCommand::new(&bin, |cmd| {
                            cmd.arg(file)
                                .arg("-T")
                                .arg(&target_profile)
                                .arg("-E")
                                .arg(&entry_point)
                                .args(params)
                                .stdout(Stdio::null())
                        })
                        .success()
                        .with_context(|| {
                            format!(
                                "failed to validate entry point {entry_point:?} with profile \
                                {target_profile:?}"
                            )
                        })
                    };
                    match cmd {
                        ValidateHlslCommand::Dxc => {
                            let bin = "dxc";
                            which(bin)?;
                            visit_hlsl(&mut |file, config_item| {
                                // Reference:
                                // <https://github.com/microsoft/DirectXShaderCompiler/blob/6ee4074a4b43fa23bf5ad27e4f6cafc6b835e437/tools/clang/docs/UsingDxc.rst>.
                                validate(
                                    bin,
                                    file,
                                    config_item,
                                    &["-Wno-parentheses-equality", "-Zi", "-Qembed_debug", "-Od"],
                                )
                            });
                        }
                        ValidateHlslCommand::Fxc => {
                            let bin = "fxc";
                            which(bin)?;
                            visit_hlsl(&mut |file, config_item| {
                                let Some(Ok(shader_model_major_version)) = config_item
                                    .target_profile
                                    .split('_')
                                    .nth(1)
                                    .map(|segment| segment.parse::<u8>()) else {
                                        bail!(
                                            "expected target profile of the form \
                                            `{{model}}_{{major}}_{{minor}}`, found invalid target \
                                            profile {:?} in file {}",
                                            config_item.target_profile,
                                            file.display()
                                        )
                                    };
                                // NOTE: This isn't implemented by `fxc.exe`; see
                                // <https://learn.microsoft.com/en-us/windows/win32/direct3dtools/dx-graphics-tools-fxc-syntax#profiles>.
                                if shader_model_major_version < 6 {
                                    // Reference:
                                    // <https://learn.microsoft.com/en-us/windows/win32/direct3dtools/dx-graphics-tools-fxc-syntax>.
                                    validate(bin, file, config_item, &["-Zi", "-Od"])
                                } else {
                                    log::debug!(
                                        "skipping config. item {config_item:?} because the \
                                        shader model major version is > 6"
                                    );
                                    Ok(())
                                }
                            });
                        }
                    }
                }
            }
            ensure!(
                !found_err,
                "failed to validate one or more files, see above output for more details"
            );
            Ok(())
        }
    }
}
