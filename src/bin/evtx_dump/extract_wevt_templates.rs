use anyhow::{Context, Result, bail};
use clap::{Arg, ArgAction, ArgMatches, Command};
use indoc::indoc;

pub fn command() -> Command {
    Command::new("extract-wevt-templates")
        .about("Build a WEVT template cache from PE files (EXE/DLL)")
        .long_about(indoc!(r#"
            Extract `WEVT_TEMPLATE` resources from PE files (EXE/DLL/SYS) and write them into a
            single portable cache file (`.wevtcache`).

            This cache can be copied across machines/OSes and used for offline template fallback
            when EVTX embedded templates are missing/corrupt.
        "#))
        .arg(
            Arg::new("input")
                .long("input")
                .short('i')
                .action(ArgAction::Append)
                .value_name("PATH")
                .help("Input PE path (file or directory). Can be passed multiple times."),
        )
        .arg(
            Arg::new("glob")
                .long("glob")
                .action(ArgAction::Append)
                .value_name("PATTERN")
                .help("Glob pattern to expand into input paths (cross-platform). Can be passed multiple times."),
        )
        .arg(
            Arg::new("recursive")
                .long("recursive")
                .short('r')
                .action(ArgAction::SetTrue)
                .help("When an input path is a directory (or a glob matches a directory), recurse into it."),
        )
        .arg(
            Arg::new("extensions")
                .long("extensions")
                .value_name("EXTS")
                .default_value("exe,dll,sys")
                .help("Comma-separated list of allowed file extensions when walking directories (default: exe,dll,sys)."),
        )
        .arg(
            Arg::new("output")
                .long("output")
                .short('o')
                .required(true)
                .value_name("WEVTCACHE")
                .help("Output `.wevtcache` file to write extracted resources into."),
        )
        .arg(
            Arg::new("overwrite")
                .long("overwrite")
                .action(ArgAction::SetTrue)
                .help("Overwrite the output file if it already exists."),
        )
}

pub fn run(matches: &ArgMatches) -> Result<()> {
    #[cfg(feature = "wevt_templates")]
    {
        imp::run_impl(matches)
    }

    #[cfg(not(feature = "wevt_templates"))]
    {
        let _ = matches;
        bail!(
            "This subcommand requires building `evtx_dump` with template support enabled.\n\
             Example:\n\
              cargo run --bin evtx_dump --features wevt_templates -- extract-wevt-templates ..."
        );
    }
}

#[cfg(feature = "wevt_templates")]
mod imp {
    use super::*;
    use evtx::wevt_templates::extract_wevt_template_resources;
    use std::collections::{HashSet, VecDeque};
    use std::fs;
    use std::path::{Path, PathBuf};

    pub(super) fn run_impl(matches: &ArgMatches) -> Result<()> {
        let output = PathBuf::from(matches.get_one::<String>("output").expect("required"));
        if output.extension().and_then(|s| s.to_str()) != Some("wevtcache") {
            bail!("output must have `.wevtcache` extension");
        }

        let overwrite = matches.get_flag("overwrite");
        let recursive = matches.get_flag("recursive");

        let allowed_exts: HashSet<String> = matches
            .get_one::<String>("extensions")
            .expect("has default")
            .split(',')
            .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();

        let mut inputs: Vec<PathBuf> = vec![];

        if let Some(paths) = matches.get_many::<String>("input") {
            inputs.extend(paths.map(PathBuf::from));
        }

        if let Some(patterns) = matches.get_many::<String>("glob") {
            for pat in patterns {
                for entry in
                    glob::glob(pat).with_context(|| format!("invalid glob pattern `{pat}`"))?
                {
                    match entry {
                        Ok(p) => inputs.push(p),
                        Err(e) => eprintln!("glob entry error: {e}"),
                    }
                }
            }
        }

        if inputs.is_empty() {
            bail!("No inputs provided. Use --input and/or --glob.");
        }

        // Expand directories (optionally recursively) and filter by extension.
        let mut files = vec![];
        let mut seen = HashSet::<PathBuf>::new();
        for input in inputs {
            collect_input_paths(&input, recursive, &allowed_exts, &mut seen, &mut files)?;
        }
        files.sort();

        let mut writer = evtx::wevt_templates::wevtcache::WevtCacheWriter::create(
            &output, overwrite,
        )?;

        let mut error_count = 0usize;
        let mut written = 0u32;

        for path in files {
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    error_count += 1;
                    eprintln!("failed to read `{}`: {e}", path.to_string_lossy());
                    continue;
                }
            };

            let resources = match extract_wevt_template_resources(&bytes) {
                Ok(r) => r,
                Err(e) => {
                    error_count += 1;
                    eprintln!(
                        "failed to extract WEVT_TEMPLATE from `{}`: {e}",
                        path.to_string_lossy()
                    );
                    continue;
                }
            };

            for res in resources {
                if let Err(e) = writer.write_crim_blob(&res.data) {
                    error_count += 1;
                    eprintln!(
                        "failed to write blob from `{}`: {e}",
                        path.to_string_lossy()
                    );
                    continue;
                }
                written = written.saturating_add(1);
            }
        }

        let _ = writer.finish()?;

        if error_count > 0 {
            bail!("extract-wevt-templates completed with {error_count} error(s)");
        }

        eprintln!("wrote {written} resource blob(s) to `{}`", output.display());
        Ok(())
    }

    fn collect_input_paths(
        input: &Path,
        recursive: bool,
        allowed_exts: &HashSet<String>,
        seen: &mut HashSet<PathBuf>,
        out_files: &mut Vec<PathBuf>,
    ) -> Result<()> {
        if !input.exists() {
            return Ok(());
        }

        if input.is_file() {
            // For explicit files (or glob matches that are files), do not apply extension filtering.
            let p = input.to_path_buf();
            if seen.insert(p.clone()) {
                out_files.push(p);
            }
            return Ok(());
        }

        if input.is_dir() {
            if !recursive {
                // Directory input without recursion is ambiguous; ignore silently.
                return Ok(());
            }

            let mut queue = VecDeque::new();
            queue.push_back(input.to_path_buf());

            while let Some(dir) = queue.pop_front() {
                let entries = fs::read_dir(&dir)
                    .with_context(|| format!("failed to read directory `{}`", dir.display()))?;
                for entry in entries {
                    let entry = entry?;
                    let p = entry.path();
                    if p.is_dir() {
                        queue.push_back(p);
                    } else if p.is_file() && should_keep_file(&p, allowed_exts) && seen.insert(p.clone()) {
                        out_files.push(p);
                    }
                }
            }
        }

        Ok(())
    }

    fn should_keep_file(path: &Path, allowed_exts: &HashSet<String>) -> bool {
        let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
            return false;
        };
        allowed_exts.contains(&ext.to_ascii_lowercase())
    }
}

