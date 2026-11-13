//! WhippyFlight tuning pipeline for the WhippyFlight tab.
//!
//! The regression (`fit_model.py`) and gain computation (`gen_wfg.py`) are
//! Python scripts run under a user-chosen interpreter from a user-chosen tools
//! directory. The webview never supplies a command line: it names a job and
//! its typed parameters, and the arguments are built here. Every file the
//! pipeline reads or writes sits at a fixed path inside one working directory:
//!
//! ```text
//! <workdir>/logs/*.bbl          flight logs pulled from the FC
//! <workdir>/flights.json        the flights in each log (`fit_model.py flights`)
//! <workdir>/cli_dump.txt        `dump`, for fit_model's motor-layout check
//! <workdir>/fits/<fit>/         one fit per selection of flights:
//!     coefficients.json, diagnostics.json, plots/*.png, noise.bin, and
//!     front/: gains over a grid of LQR weights (front.json, <n>.wfg) and
//!     their scores (scores.json, `whippyflight_front`)
//! <workdir>/gains/gains.wfg     gains, and gains/fit.txt the fit they came from
//! <workdir>/gains/flicks.json   the gains flown through stick flicks on that
//!     fit's plant (`whippyflight_flicks`), and gains/plots/*.png of them
//! ```
//!
//! Script output is streamed line by line on the `whippyflight-output` event.
//! The flicks are flown here, in process, by the controller crate itself.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{AppHandle, Emitter, State};

const FIT_SCRIPT: &str = "fit_model.py";
const GEN_SCRIPT: &str = "gen_wfg.py";
const LOGS_DIR: &str = "logs";
const FLIGHT_INDEX: &str = "flights.json";
const CLI_DUMP: &str = "cli_dump.txt";
const FITS_DIR: &str = "fits";
const COEFFICIENTS: &str = "coefficients.json";
const DIAGNOSTICS: &str = "diagnostics.json";
const PLOTS_DIR: &str = "plots";
const GAINS: &str = "gains/gains.wfg";
const GAINS_FIT: &str = "gains/fit.txt";
const GAINS_WEIGHTS: &str = "gains/weights.json";
const FLICKS: &str = "gains/flicks.json";
const FLICK_PLOTS_DIR: &str = "gains/plots";
const FLICK_PLOT_SCRIPT: &str = "plot_flicks.py";
const FIT_PLOT_SCRIPT: &str = "fit_diagnostics.py";
const NOISE_SCRIPT: &str = "extract_noise.py";
/// A fit's sensor noise (extract_noise.py), and its window and BF's roughness there.
const NOISE: &str = "noise.bin";
const NOISE_INFO: &str = "noise.json";
const FRONT_SCRIPT: &str = "front_wfg.py";
/// A fit's Pareto front: the blobs front_wfg.py computed, their index, and their scores.
const FRONT_DIR: &str = "front";
const FRONT_INDEX: &str = "front.json";
const FRONT_SCORES: &str = "scores.json";
/// Flick traces are kept at this spacing (or the loop period, if longer).
const FLICK_TRACE_DT_S: f64 = 0.0005;
const REQUIRED_MODULES: &[&str] = &["numpy", "scipy", "control", "frccontrol"];

#[derive(Default)]
pub struct WhippyFlightState {
    child: Arc<Mutex<Option<Child>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Toolchain {
    python: String,
    tools_dir: String,
}

/// gen_wfg.py's LQR cost weights (Bryson: the excursion each state may make).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Weights {
    /// Rate excursion, rot/s.
    sigma_rate: f64,
    /// Derivative-state excursion, rot/s².
    sigma_deriv: f64,
    /// Low-passed rate error the integral holds, rot/s.
    sigma_int: f64,
}

/// The motor-speed grid gen_wfg.py schedules the gains over.
#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Grid {
    /// Speeds per motor, N: the blob holds N⁴ points.
    resolution: u8,
    /// The FC's `whippyflight_idle_rps`, rev/s; the grid starts there when it is above
    /// gen_wfg's minimum. 0 for none.
    idle_rps: f64,
}

/// Grids up to this resolution fit the firmware's gain store (`WHIPPYFLIGHT_GAIN_STORE_SIZE`).
const MAX_RESOLUTION: u8 = 4;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Job {
    /// Index the flights in these logs (file names in `<workdir>/logs`) into `flights.json`.
    #[serde(rename_all = "camelCase")]
    Scan { logs: Vec<String> },
    #[serde(rename_all = "camelCase")]
    Fit {
        /// `<log>` for every flight in a log, or `<log>#<n>` for its n-th (1-based).
        flights: Vec<String>,
        /// "auto", "on" or "off".
        drag: String,
        /// Directory name under `fits/`; replaced if it exists.
        fit: String,
    },
    #[serde(rename_all = "camelCase")]
    Gen {
        /// Loop period, s: the FC's; the fit's logged loop rate if None.
        dt: Option<f64>,
        fit: String,
        weights: Weights,
        grid: Grid,
    },
    /// Extract the sensor noise of `flight` (a fit spec) into `fits/<fit>/noise.bin`.
    #[serde(rename_all = "camelCase")]
    Noise { fit: String, flight: String },
    /// Plot `fits/<fit>/diagnostics.json` into `fits/<fit>/plots`.
    #[serde(rename_all = "camelCase")]
    FitPlots { fit: String },
    /// Compute gains over a grid of σ_deriv and σ_int into `fits/<fit>/front`.
    #[serde(rename_all = "camelCase")]
    Front {
        /// As [`Job::Gen`]'s.
        dt: Option<f64>,
        fit: String,
        sigma_rate: f64,
        grid: Grid,
    },
    /// Plot `gains/flicks.json` into `gains/plots`.
    #[serde(rename_all = "camelCase")]
    FlickPlots,
}

#[derive(Serialize, Clone)]
struct OutputLine {
    line: String,
    stderr: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvReport {
    python_version: Option<String>,
    missing_modules: Vec<String>,
    has_matplotlib: bool,
    missing_scripts: Vec<String>,
}

#[derive(Serialize)]
pub struct LogFile {
    name: String,
    size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FitRun {
    id: String,
    has_coefficients: bool,
    has_diagnostics: bool,
    has_noise: bool,
    /// Whether its Pareto front has been scored.
    has_front: bool,
    plots: Vec<String>,
    /// When its coefficients were written, ms since the epoch; 0 if they weren't.
    modified: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    logs: Vec<LogFile>,
    has_flight_index: bool,
    has_cli_dump: bool,
    fits: Vec<FitRun>,
    has_gains: bool,
    /// The fit the gains were computed from.
    gains_fit: Option<String>,
    /// The weights the gains were computed with.
    gains_weights: Option<Weights>,
    has_flicks: bool,
    flick_plots: Vec<String>,
}

fn workdir(path: &str) -> Result<PathBuf, String> {
    let dir = PathBuf::from(path);
    if !dir.is_absolute() || !dir.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    Ok(dir)
}

/// A bare file name: no separators, no `..`, nothing hidden.
fn plain_name(name: &str) -> Result<&str, String> {
    let ok = !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
        && Path::new(name).file_name().and_then(|n| n.to_str()) == Some(name);
    ok.then_some(name)
        .ok_or_else(|| format!("invalid file name: {name}"))
}

/// A FIT_SCRIPT flight spec: a log name, optionally `#<n>` with n ≥ 1.
fn flight_spec(spec: &str) -> Result<(&str, Option<u32>), String> {
    let (name, index) = match spec.rsplit_once('#') {
        Some((name, text)) => {
            let n = text
                .parse::<u32>()
                .ok()
                .filter(|&n| n > 0 && n.to_string() == text)
                .ok_or_else(|| format!("invalid flight: {spec}"))?;
            (name, Some(n))
        }
        None => (spec, None),
    };
    Ok((plain_name(name)?, index))
}

/// Resolve one of the pipeline's fixed relative paths, a log by name, or a
/// fit's file (`fits/<fit>/coefficients.json`, `…/diagnostics.json`, `…/plots/<name>`).
fn resolve(dir: &Path, file: &str) -> Result<PathBuf, String> {
    let unknown = || format!("unknown file: {file}");
    match file {
        CLI_DUMP | FLIGHT_INDEX | GAINS | GAINS_FIT | FLICKS => return Ok(dir.join(file)),
        _ => {}
    }
    let parts: Vec<&str> = file.split('/').collect();
    match parts.as_slice() {
        [LOGS_DIR, name] => Ok(dir.join(LOGS_DIR).join(plain_name(name)?)),
        [FITS_DIR, fit, name @ (COEFFICIENTS | DIAGNOSTICS)] => {
            Ok(dir.join(FITS_DIR).join(plain_name(fit)?).join(name))
        }
        [FITS_DIR, fit, FRONT_DIR, FRONT_SCORES] => Ok(dir
            .join(FITS_DIR)
            .join(plain_name(fit)?)
            .join(FRONT_DIR)
            .join(FRONT_SCORES)),
        [FITS_DIR, fit, PLOTS_DIR, name] => Ok(dir
            .join(FITS_DIR)
            .join(plain_name(fit)?)
            .join(PLOTS_DIR)
            .join(plain_name(name)?)),
        _ => Err(unknown()),
    }
}

fn modified_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

fn list_names(dir: &Path, extension: &str) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let meta = entry.metadata().ok()?;
            (meta.is_file() && !name.starts_with('.') && name.to_lowercase().ends_with(extension))
                .then_some((name, meta.len()))
        })
        .collect();
    out.sort();
    out
}

fn python_command(toolchain: &Toolchain) -> Command {
    let mut command = Command::new(&toolchain.python);
    command
        .current_dir(&toolchain.tools_dir)
        .env("PYTHONUNBUFFERED", "1")
        .env("MPLBACKEND", "Agg")
        .stdin(Stdio::null());
    command
}

fn script(toolchain: &Toolchain, name: &str) -> String {
    Path::new(&toolchain.tools_dir)
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn check_dt(dt: Option<f64>) -> Result<(), String> {
    match dt {
        Some(dt) if !(1e-5..1e-2).contains(&dt) => {
            Err(format!("implausible PID loop period: {dt} s"))
        }
        _ => Ok(()),
    }
}

fn check_sigma(flag: &str, sigma: f64) -> Result<(), String> {
    if sigma.is_finite() && sigma > 0.0 {
        Ok(())
    } else {
        Err(format!("{flag} must be positive, not {sigma}"))
    }
}

/// gen_wfg.py's arguments for `grid`.
fn grid_args(grid: &Grid) -> Result<Vec<String>, String> {
    if !(2..=MAX_RESOLUTION).contains(&grid.resolution) {
        return Err(format!(
            "grid resolution must be 2–{MAX_RESOLUTION}, not {}",
            grid.resolution
        ));
    }
    if !(grid.idle_rps.is_finite() && grid.idle_rps >= 0.0) {
        return Err(format!("implausible idle speed: {} rev/s", grid.idle_rps));
    }
    Ok(vec![
        "--resolution".into(),
        grid.resolution.to_string(),
        "--idle-rps".into(),
        format!("{:e}", grid.idle_rps),
    ])
}

fn job_args(toolchain: &Toolchain, dir: &Path, job: &Job) -> Result<Vec<String>, String> {
    let text = |path: PathBuf| path.to_string_lossy().into_owned();
    let log = |name: &str| -> Result<String, String> {
        Ok(text(dir.join(LOGS_DIR).join(plain_name(name)?)))
    };
    match job {
        Job::Scan { logs } => {
            if logs.is_empty() {
                return Err("no logs to scan".into());
            }
            let mut args = vec![script(toolchain, FIT_SCRIPT), "flights".into()];
            for name in logs {
                args.push(log(name)?);
            }
            args.extend(["-o".into(), text(dir.join(FLIGHT_INDEX))]);
            Ok(args)
        }
        Job::Fit { flights, drag, fit } => {
            if flights.is_empty() {
                return Err("no flights to fit".into());
            }
            if !matches!(drag.as_str(), "auto" | "on" | "off") {
                return Err(format!("invalid drag mode: {drag}"));
            }
            let mut args = vec![script(toolchain, FIT_SCRIPT), "fit".into()];
            for spec in flights {
                let (name, index) = flight_spec(spec)?;
                let path = log(name)?;
                args.push(index.map_or(path.clone(), |n| format!("{path}#{n}")));
            }
            let out = dir.join(FITS_DIR).join(plain_name(fit)?);
            // A stale diagnostics.json or plot must not outlive the fit it described.
            let _ = std::fs::remove_dir_all(&out);
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            args.extend([
                "-o".into(),
                text(out.join(COEFFICIENTS)),
                "--diagnostics".into(),
                text(out.join(DIAGNOSTICS)),
                "--drag".into(),
                drag.clone(),
            ]);
            if dir.join(CLI_DUMP).is_file() {
                args.extend(["--dump".into(), text(dir.join(CLI_DUMP))]);
            }
            Ok(args)
        }
        Job::Gen {
            dt,
            fit,
            weights,
            grid,
        } => {
            check_dt(*dt)?;
            let grid = grid_args(grid)?;
            let coefficients = dir.join(FITS_DIR).join(plain_name(fit)?).join(COEFFICIENTS);
            if !coefficients.is_file() {
                return Err(format!("no fit {fit} to compute gains from"));
            }
            std::fs::create_dir_all(dir.join("gains")).map_err(|e| e.to_string())?;
            let mut args = vec![
                script(toolchain, GEN_SCRIPT),
                text(coefficients),
                text(dir.join(GAINS)),
            ];
            if let Some(dt) = dt {
                args.extend(["--dt".into(), format!("{dt:e}")]);
            }
            for (flag, sigma) in [
                ("--sigma-rate", weights.sigma_rate),
                ("--sigma-deriv", weights.sigma_deriv),
                ("--sigma-int", weights.sigma_int),
            ] {
                check_sigma(flag, sigma)?;
                args.extend([flag.into(), format!("{sigma:e}")]);
            }
            args.extend(grid);
            Ok(args)
        }
        Job::Noise { fit, flight } => {
            let out = dir.join(FITS_DIR).join(plain_name(fit)?);
            if !out.is_dir() {
                return Err(format!("no fit {fit}"));
            }
            let (name, index) = flight_spec(flight)?;
            let path = log(name)?;
            Ok(vec![
                script(toolchain, NOISE_SCRIPT),
                index.map_or(path.clone(), |n| format!("{path}#{n}")),
                text(out.join(NOISE)),
                "--json".into(),
                text(out.join(NOISE_INFO)),
            ])
        }
        Job::FitPlots { fit } => {
            let out = dir.join(FITS_DIR).join(plain_name(fit)?);
            if !out.join(DIAGNOSTICS).is_file() {
                return Err(format!("no diagnostics for fit {fit} to plot"));
            }
            let _ = std::fs::remove_dir_all(out.join(PLOTS_DIR));
            Ok(vec![
                script(toolchain, FIT_PLOT_SCRIPT),
                text(out.join(DIAGNOSTICS)),
                text(out.join(PLOTS_DIR)),
            ])
        }
        Job::Front {
            dt,
            fit,
            sigma_rate,
            grid,
        } => {
            check_dt(*dt)?;
            check_sigma("--sigma-rate", *sigma_rate)?;
            let grid = grid_args(grid)?;
            let out = dir.join(FITS_DIR).join(plain_name(fit)?);
            if !out.join(COEFFICIENTS).is_file() {
                return Err(format!("no fit {fit} to compute gains from"));
            }
            let _ = std::fs::remove_dir_all(out.join(FRONT_DIR));
            let mut args = vec![
                script(toolchain, FRONT_SCRIPT),
                text(out.join(COEFFICIENTS)),
                text(out.join(FRONT_DIR)),
                "--sigma-rate".into(),
                format!("{sigma_rate:e}"),
            ];
            if let Some(dt) = dt {
                args.extend(["--dt".into(), format!("{dt:e}")]);
            }
            args.extend(grid);
            Ok(args)
        }
        Job::FlickPlots => {
            if !dir.join(FLICKS).is_file() {
                return Err("no flicks to plot".into());
            }
            let _ = std::fs::remove_dir_all(dir.join(FLICK_PLOTS_DIR));
            Ok(vec![
                script(toolchain, FLICK_PLOT_SCRIPT),
                text(dir.join(FLICKS)),
                text(dir.join(FLICK_PLOTS_DIR)),
            ])
        }
    }
}

fn forward<R: Read + Send + 'static>(
    app: AppHandle,
    stream: R,
    stderr: bool,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            let _ = app.emit("whippyflight-output", OutputLine { line, stderr });
        }
    })
}

/// Python version, missing modules and missing scripts for a toolchain.
#[tauri::command]
pub async fn whippyflight_check_env(toolchain: Toolchain) -> Result<EnvReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let missing_scripts = [
            FIT_SCRIPT,
            GEN_SCRIPT,
            FLICK_PLOT_SCRIPT,
            FIT_PLOT_SCRIPT,
            NOISE_SCRIPT,
            FRONT_SCRIPT,
            "bbl.py",
        ]
        .iter()
        .filter(|s| !Path::new(&toolchain.tools_dir).join(s).is_file())
        .map(|s| s.to_string())
        .collect();
        let probe = "import importlib.util, sys\n\
                     print(sys.version.split()[0])\n\
                     for m in sys.argv[1:]:\n    print(m, importlib.util.find_spec(m) is not None)";
        let output = Command::new(&toolchain.python)
            .arg("-c")
            .arg(probe)
            .args(REQUIRED_MODULES)
            .arg("matplotlib")
            .stdin(Stdio::null())
            .output();
        let Ok(output) = output
            .as_ref()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        else {
            return EnvReport {
                python_version: None,
                missing_modules: REQUIRED_MODULES.iter().map(|m| m.to_string()).collect(),
                has_matplotlib: false,
                missing_scripts,
            };
        };
        let mut lines = output.lines();
        let python_version = lines.next().map(str::to_owned);
        let present: Vec<(String, bool)> = lines
            .filter_map(|l| {
                l.split_once(' ')
                    .map(|(m, ok)| (m.to_owned(), ok == "True"))
            })
            .collect();
        let found = |m: &str| present.iter().any(|(name, ok)| name == m && *ok);
        EnvReport {
            python_version,
            missing_modules: REQUIRED_MODULES
                .iter()
                .filter(|m| !found(m))
                .map(|m| m.to_string())
                .collect(),
            has_matplotlib: found("matplotlib"),
            missing_scripts,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

/// Forget what was flown or recorded for the gains, which are about to be replaced.
fn clear_gains_records(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(GAINS_FIT));
    let _ = std::fs::remove_file(dir.join(GAINS_WEIGHTS));
    let _ = std::fs::remove_file(dir.join(FLICKS));
    let _ = std::fs::remove_dir_all(dir.join(FLICK_PLOTS_DIR));
}

/// Record the fit and weights new gains were computed from.
fn record_gains(dir: &Path, fit: &str, weights: &Weights) -> Result<(), String> {
    let json = serde_json::to_vec(weights).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(GAINS_WEIGHTS), json).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(GAINS_FIT), fit).map_err(|e| e.to_string())
}

/// Clear what a job's outputs supersede, before it runs.
fn before_job(dir: &Path, job: &Job) {
    match job {
        Job::Gen { .. } => clear_gains_records(dir),
        Job::Fit { fit, .. } => {
            if let Ok(name) = plain_name(fit) {
                let out = dir.join(FITS_DIR).join(name);
                let _ = std::fs::remove_file(out.join(NOISE));
                let _ = std::fs::remove_file(out.join(NOISE_INFO));
            }
        }
        _ => {}
    }
}

/// Record what a job that succeeded produced from.
fn after_job(dir: &Path, job: &Job) -> Result<(), String> {
    match job {
        Job::Gen { fit, weights, .. } => record_gains(dir, fit, weights),
        _ => Ok(()),
    }
}

/// Run one pipeline job to completion; resolves with the script's exit code.
#[tauri::command]
pub async fn whippyflight_run(
    app: AppHandle,
    state: State<'_, WhippyFlightState>,
    toolchain: Toolchain,
    workdir: String,
    job: Job,
) -> Result<i32, String> {
    let dir = self::workdir(&workdir)?;
    let args = job_args(&toolchain, &dir, &job)?;
    before_job(&dir, &job);
    let slot = state.child.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut command = python_command(&toolchain);
        command
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let readers = {
            let mut guard = slot.lock().unwrap();
            if guard.is_some() {
                return Err("a WhippyFlight job is already running".to_string());
            }
            let mut child = command
                .spawn()
                .map_err(|e| format!("could not start {}: {e}", toolchain.python))?;
            let readers = [
                forward(app.clone(), child.stdout.take().unwrap(), false),
                forward(app.clone(), child.stderr.take().unwrap(), true),
            ];
            *guard = Some(child);
            readers
        };
        let status = loop {
            let mut guard = slot.lock().unwrap();
            match guard.as_mut().map(Child::try_wait) {
                Some(Ok(Some(status))) => {
                    guard.take();
                    break Ok(status);
                }
                Some(Ok(None)) => {}
                Some(Err(e)) => {
                    guard.take();
                    break Err(e.to_string());
                }
                None => break Err("cancelled".to_string()),
            }
            drop(guard);
            thread::sleep(std::time::Duration::from_millis(50));
        };
        for reader in readers {
            let _ = reader.join();
        }
        let code = status?.code().unwrap_or(-1);
        if code == 0 {
            after_job(&dir, &job)?;
        }
        Ok(code)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Kill the running job, if any; its `whippyflight_run` rejects with "cancelled".
#[tauri::command]
pub fn whippyflight_cancel(state: State<'_, WhippyFlightState>) {
    if let Some(mut child) = state.child.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// What the working directory holds so far.
#[tauri::command]
pub fn whippyflight_workspace(workdir: String) -> Result<Workspace, String> {
    let dir = self::workdir(&workdir)?;
    let mut fits: Vec<FitRun> = std::fs::read_dir(dir.join(FITS_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let id = entry.file_name().into_string().ok()?;
            let path = entry.path();
            (path.is_dir() && plain_name(&id).is_ok()).then(|| FitRun {
                has_coefficients: path.join(COEFFICIENTS).is_file(),
                has_diagnostics: path.join(DIAGNOSTICS).is_file(),
                has_noise: path.join(NOISE).is_file(),
                has_front: path.join(FRONT_DIR).join(FRONT_SCORES).is_file(),
                plots: list_names(&path.join(PLOTS_DIR), ".png")
                    .into_iter()
                    .map(|(n, _)| n)
                    .collect(),
                modified: modified_ms(&path.join(COEFFICIENTS)),
                id,
            })
        })
        .collect();
    fits.sort_by(|a, b| a.id.cmp(&b.id));
    let mut logs: Vec<LogFile> = list_names(&dir.join(LOGS_DIR), ".bbl")
        .into_iter()
        .chain(list_names(&dir.join(LOGS_DIR), ".bfl"))
        .map(|(name, size)| LogFile { name, size })
        .collect();
    logs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Workspace {
        logs,
        has_flight_index: dir.join(FLIGHT_INDEX).is_file(),
        has_cli_dump: dir.join(CLI_DUMP).is_file(),
        fits,
        has_gains: dir.join(GAINS).is_file(),
        gains_fit: std::fs::read_to_string(dir.join(GAINS_FIT))
            .ok()
            .map(|s| s.trim().to_owned()),
        gains_weights: std::fs::read(dir.join(GAINS_WEIGHTS))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok()),
        has_flicks: dir.join(FLICKS).is_file(),
        flick_plots: list_names(&dir.join(FLICK_PLOTS_DIR), ".png")
            .into_iter()
            .map(|(n, _)| n)
            .collect(),
    })
}

/// Read a pipeline file (see `resolve`) as raw bytes.
#[tauri::command]
pub fn whippyflight_read(workdir: String, file: String) -> Result<Response, String> {
    let path = resolve(&self::workdir(&workdir)?, &file)?;
    std::fs::read(&path)
        .map(Response::new)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Write a log or the CLI dump. The body is the raw file contents; the
/// `workdir` and `file` headers (percent-encoded) say where.
#[tauri::command]
pub fn whippyflight_write(request: Request<'_>) -> Result<(), String> {
    let header = |name: &str| -> Result<String, String> {
        let raw = request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| format!("missing {name} header"))?;
        percent_decode(raw)
    };
    let dir = workdir(&header("workdir")?)?;
    let file = header("file")?;
    if file != CLI_DUMP && !file.starts_with("logs/") {
        return Err(format!("not writable: {file}"));
    }
    let path = resolve(&dir, &file)?;
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected a raw body".into());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Where an imported log goes: its own name in `logs`, or the name with a
/// `_2`, `_3`, … suffix if a different file has it; and whether it's already there.
fn import_target(logs: &Path, name: &str, bytes: &[u8]) -> Result<(PathBuf, bool), String> {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    for n in 1.. {
        let candidate = if n == 1 {
            name.to_owned()
        } else {
            format!("{stem}_{n}{ext}")
        };
        let path = logs.join(plain_name(&candidate)?);
        match std::fs::read(&path) {
            Ok(existing) if existing == bytes => return Ok((path, true)),
            Ok(_) => continue,
            Err(_) => return Ok((path, false)),
        }
    }
    unreachable!()
}

/// Copy files the user picked into the working directory: blackbox logs into
/// `logs` (returns the names they got), or a CLI dump as `cli_dump.txt`.
#[tauri::command]
pub fn whippyflight_import(
    workdir: String,
    paths: Vec<String>,
    dump: bool,
) -> Result<Vec<String>, String> {
    let dir = self::workdir(&workdir)?;
    let read = |path: &str| std::fs::read(path).map_err(|e| format!("{path}: {e}"));
    if dump {
        let [path] = paths.as_slice() else {
            return Err("pick one CLI dump".into());
        };
        std::fs::write(dir.join(CLI_DUMP), read(path)?).map_err(|e| e.to_string())?;
        return Ok(vec![CLI_DUMP.into()]);
    }
    let logs = dir.join(LOGS_DIR);
    std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
    let mut names = vec![];
    for path in &paths {
        let bytes = read(path)?;
        let name = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("{path}: no file name"))?;
        let (target, present) = import_target(&logs, name, &bytes)?;
        if !present {
            std::fs::write(&target, &bytes).map_err(|e| format!("{}: {e}", target.display()))?;
        }
        names.push(target.file_name().unwrap().to_string_lossy().into_owned());
    }
    Ok(names)
}

/// Delete one log from `<workdir>/logs`.
#[tauri::command]
pub fn whippyflight_delete_log(workdir: String, name: String) -> Result<(), String> {
    let path = self::workdir(&workdir)?
        .join(LOGS_DIR)
        .join(plain_name(&name)?);
    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Delete one fit from `<workdir>/fits`.
#[tauri::command]
pub fn whippyflight_delete_fit(workdir: String, fit: String) -> Result<(), String> {
    let path = self::workdir(&workdir)?
        .join(FITS_DIR)
        .join(plain_name(&fit)?);
    std::fs::remove_dir_all(&path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The working directory, or `logs`, `fits/<fit>` or `fits/<fit>/plots` in it.
fn reveal_path(dir: &Path, sub: Option<&str>) -> Result<PathBuf, String> {
    let Some(sub) = sub else {
        return Ok(dir.to_path_buf());
    };
    match sub.split('/').collect::<Vec<_>>().as_slice() {
        [LOGS_DIR] => Ok(dir.join(LOGS_DIR)),
        ["gains", PLOTS_DIR] => Ok(dir.join(FLICK_PLOTS_DIR)),
        [FITS_DIR, fit] => Ok(dir.join(FITS_DIR).join(plain_name(fit)?)),
        [FITS_DIR, fit, PLOTS_DIR] => Ok(dir.join(FITS_DIR).join(plain_name(fit)?).join(PLOTS_DIR)),
        _ => Err(format!("unknown directory: {sub}")),
    }
}

/// Show the working directory (or a subdirectory of it) in the OS file manager.
#[tauri::command]
pub fn whippyflight_reveal(workdir: String, sub: Option<String>) -> Result<(), String> {
    let path = reveal_path(&self::workdir(&workdir)?, sub.as_deref())?;
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    Command::new(opener)
        .arg(&path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// One flick (see `whippyflight_control::sim::flick`): ratios are of `amp`,
/// times in ms; `rise90_ms` is null when 90% was never reached.
#[derive(Serialize, Deserialize, Debug)]
pub struct FlickMetrics {
    target_ovs: f64,
    target_lag_ms: f64,
    gyro_lag_ms: f64,
    rise90_ms: Option<f64>,
    overshoot: f64,
    hold_err: f64,
    track_rms: f64,
    residual: f64,
    sat_frac: f64,
    /// Motor roughness from the fit's sensor noise, V; None without its noise.
    rough_v: Option<f64>,
}

/// One flick's traces from the stick's start, in rot/s every `trace_dt` s:
/// `response` is the gyro with the stick minus without it.
#[derive(Serialize, Deserialize)]
pub struct FlickResult {
    /// "roll" or "pitch".
    axis: String,
    rise_ms: f64,
    amp: f64,
    metrics: FlickMetrics,
    stick: Vec<f32>,
    setpoint: Vec<f32>,
    target: Vec<f32>,
    response: Vec<f32>,
    /// Ticks within each trace sample the motors saturated, as a fraction.
    saturated: Vec<f32>,
    /// Commanded voltage of each motor (noise-free), V.
    volts: [Vec<f32>; 4],
}

/// `gains/flicks.json`: one full-stick flick per axis, the fastest raised
/// cosine within the gains' target slew and jerk limits.
#[derive(Serialize, Deserialize)]
pub struct FlickReport {
    version: u32,
    /// The fit whose plant the gains were flown on.
    fit: String,
    loop_dt: f64,
    trace_dt: f64,
    /// How long the stick holds before releasing, s after it gets there.
    hold_s: f64,
    /// The target limits, rot/s² and rot/s³.
    slew: f64,
    jerk: f64,
    /// Where the amplitudes, the full-stick rates, came from: "fc", "log"
    /// (the fit's blackbox header) or "default" (Betaflight's).
    rates_from: String,
    /// The stick estimator's tracking index flown (`whippyflight_stick_tracking`).
    stick_tracking: f32,
    /// Where it came from: "fc" (the settings panel) or "log" (the fit's blackbox header).
    stick_tracking_from: String,
    /// BF's own motor roughness, V, over the window of the log the noise came from:
    /// everything above 50 Hz in its motor outputs at hover, not only what noise caused.
    bf_rough_v: Option<f64>,
    /// What the fit's sensor noise adds to these gains' motor voltages at hover
    /// (`sim::margin::hover_roughness`), V, the like-for-like figure beside
    /// [`bf_rough_v`]; None without the noise.
    hover_rough_v: Option<f64>,
    /// Extra gyro delay, ms, the gains tolerate on top of the fit's gyro filters
    /// (`sim::margin`); −1 if they don't fly cleanly even without, None if the
    /// filters aren't modeled.
    delay_margin_ms: Option<f64>,
    /// The most [`delay_margin_ms`] reports.
    max_margin_ms: f64,
    /// Attitude, deg, a 5% CG shift costs before the integral takes up the
    /// torque (`sim::margin::trim`); None if the filters aren't modeled.
    trim_deg: Option<f64>,
    flicks: Vec<FlickResult>,
}

/// Betaflight's default max rate, rot/s (670°/s on every axis).
const DEFAULT_MAX_RATE_RPS: f64 = 670.0 / 360.0;

/// The rise, s, of the fastest raised-cosine stick to `amp` (rot/s) whose
/// peak slope Aπ/2T and slope change Aπ²/2T² stay within `slew` and `jerk`.
fn limit_rise_s(amp: f64, slew: f64, jerk: f64) -> f64 {
    use std::f64::consts::PI;
    (PI * (amp / (2.0 * jerk)).sqrt()).max(PI * amp / (2.0 * slew))
}

/// Full-stick roll, pitch and yaw rates, rot/s, and where they came from.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaxRates {
    rps: [f64; 3],
    /// "fc" or "log".
    from: String,
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The plant `fits/<fit>` fitted.
fn fit_model(dir: &Path, fit: &str) -> Result<whippyflight_control::sim::plant::Model, String> {
    let coefficients = dir.join(FITS_DIR).join(plain_name(fit)?).join(COEFFICIENTS);
    let text = String::from_utf8(read_file(&coefficients)?).map_err(|e| e.to_string())?;
    whippyflight_control::sim::plant::Model::from_json(&text, &coefficients.to_string_lossy())
}

/// A fit's sensor noise, if it has been extracted, and BF's own roughness over it.
fn fit_noise(
    dir: &Path,
    fit: &str,
) -> Result<(Option<whippyflight_control::sim::noise::Noise>, Option<f64>), String> {
    use whippyflight_control::sim::{noise::Noise, units::Time};
    let fit_dir = dir.join(FITS_DIR).join(plain_name(fit)?);
    let noise = match std::fs::read(fit_dir.join(NOISE)) {
        Ok(bytes) => Some(Noise::from_bytes(&bytes, Time::new(0.0), 1.0, 1.0)?),
        Err(_) => None,
    };
    let bf_rough_v = std::fs::read(fit_dir.join(NOISE_INFO))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["bf_rough_v"].as_f64());
    Ok((noise, bf_rough_v))
}

fn load_blob(path: &Path) -> Result<whippyflight_control::gains::GainStorage, String> {
    use whippyflight_control::gains::{GainStorage, load_gains};
    use whippyflight_control::sim::static_blob;
    use whippyflight_control::types::GainLoadResult;
    let mut gains = GainStorage::new();
    // load_gains reads the tables in place for as long as the gains are used; the blob is
    // a few kB, leaked once per simulation.
    let result = load_gains(static_blob(&read_file(path)?), &mut gains);
    if result != GainLoadResult::Ok {
        return Err(format!("{} doesn't load: {result:?}", path.display()));
    }
    Ok(gains)
}

fn fly_flicks(
    dir: &Path,
    max_rates: Option<MaxRates>,
    stick_tracking: Option<f32>,
) -> Result<FlickReport, String> {
    use whippyflight_control::setpoint::target_limits;
    use whippyflight_control::sim::units::{Rate, Time};
    use whippyflight_control::sim::{flick, margin};
    use whippyunits::value;

    let fit = std::fs::read_to_string(dir.join(GAINS_FIT))
        .map_err(|_| "it isn't recorded which fit the gains came from".to_string())?
        .trim()
        .to_owned();
    let mut model = fit_model(dir, &fit)?;
    let stick_tracking_from = match stick_tracking {
        Some(lambda) if lambda.is_finite() && lambda > 0.0 => {
            model.stick_tracking = lambda;
            "fc"
        }
        Some(lambda) => return Err(format!("bad stick tracking {lambda}")),
        None => "log",
    };
    let gains = load_blob(&dir.join(GAINS))?;
    let loop_dt = value!(gains.header.dt, s, f32) as f64;
    let stride = ((FLICK_TRACE_DT_S / loop_dt).round() as usize).max(1);
    let thin = |v: Vec<f32>| v.into_iter().step_by(stride).collect::<Vec<_>>();
    let rps = |v: &[Rate]| {
        v.iter()
            .map(|&r| value!(r, rot / s) as f32)
            .collect::<Vec<_>>()
    };
    let ms = |t: Time| value!(t, ms);
    let (rates, rates_from) = match max_rates {
        Some(MaxRates { rps, from }) if from == "fc" || from == "log" => (rps, from),
        Some(MaxRates { from, .. }) => return Err(format!("unknown rate source {from}")),
        None => ([DEFAULT_MAX_RATE_RPS; 3], "default".into()),
    };
    if rates.iter().any(|r| !(r.is_finite() && *r > 0.0)) {
        return Err(format!("bad max rates {rates:?} rot/s"));
    }
    let (noise, bf_rough_v) = fit_noise(dir, &fit)?;
    let cutoffs = margin::pt1_cutoffs(&model).ok();
    let delay_margin_ms = cutoffs
        .as_ref()
        .map(|c| value!(margin::delay_margin(&gains, &model, c), ms));
    let trim_deg = cutoffs
        .as_ref()
        .map(|c| value!(margin::trim(&gains, &model, c).0, deg));
    let hover_rough_v = noise.as_ref().map(|n| {
        let filters = cutoffs.as_deref().unwrap_or_default();
        value!(margin::hover_roughness(&gains, &model, filters, n), V)
    });
    let (slew, jerk) = target_limits(&gains);
    let (slew, jerk) = (
        value!(slew, rot / s ^ 2, f32) as f64,
        value!(jerk, rot / s ^ 3, f32) as f64,
    );
    let flicks = (0..3)
        .map(|axis| {
            let amp = Rate::new(rates[axis]);
            let rise = Time::new(limit_rise_s(rates[axis], slew, jerk));
            let f = flick::measure_with(
                &gains,
                &model,
                axis,
                amp,
                rise,
                flick::Limits::default(),
                noise.as_ref(),
            );
            let m = f.metrics;
            let saturated = f
                .run
                .saturated
                .chunks(stride)
                .map(|c| c.iter().filter(|&&s| s).count() as f32 / c.len() as f32)
                .collect();
            FlickResult {
                axis: ["roll", "pitch", "yaw"][axis].to_string(),
                rise_ms: ms(rise),
                amp: value!(amp, rot / s),
                metrics: FlickMetrics {
                    target_ovs: m.target_ovs,
                    target_lag_ms: ms(m.target_lag),
                    gyro_lag_ms: ms(m.gyro_lag),
                    rise90_ms: Some(ms(m.rise90)).filter(|t| t.is_finite()),
                    overshoot: m.overshoot,
                    hold_err: m.hold_err,
                    track_rms: m.track_rms,
                    residual: m.residual,
                    sat_frac: m.sat_frac,
                    rough_v: m.rough_v.map(|v| value!(v, V)),
                },
                stick: thin(rps(&f.run.stick)),
                setpoint: thin(rps(&f.run.setpoint)),
                target: thin(rps(&f.run.target)),
                response: thin(rps(&f.response)),
                saturated,
                volts: std::array::from_fn(|m| {
                    thin(f.run.volts.iter().map(|v| value!(v[m], V) as f32).collect())
                }),
            }
        })
        .collect();
    Ok(FlickReport {
        version: 7,
        fit,
        loop_dt,
        trace_dt: loop_dt * stride as f64,
        hold_s: value!(flick::HOLD, s),
        slew,
        jerk,
        rates_from,
        stick_tracking: model.stick_tracking,
        stick_tracking_from: stick_tracking_from.into(),
        bf_rough_v,
        hover_rough_v,
        delay_margin_ms,
        max_margin_ms: value!(margin::MAX_MARGIN, ms),
        trim_deg,
        flicks,
    })
}

/// Fly the computed gains through a full-stick flick per axis on the plant of
/// the fit they came from; writes and returns `gains/flicks.json`.
/// `stick_tracking` overrides the λ the fit's logs were flown with.
#[tauri::command]
pub async fn whippyflight_flicks(
    workdir: String,
    max_rates: Option<MaxRates>,
    stick_tracking: Option<f32>,
) -> Result<FlickReport, String> {
    let dir = self::workdir(&workdir)?;
    tauri::async_runtime::spawn_blocking(move || {
        let report = fly_flicks(&dir, max_rates, stick_tracking)?;
        let json = serde_json::to_vec(&report).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(FLICKS), json).map_err(|e| e.to_string())?;
        Ok(report)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// front_wfg.py's `front.json`.
#[derive(Deserialize)]
struct FrontIndex {
    dt: f64,
    sigma_rate: f64,
    /// Absent from fronts computed before these were recorded.
    #[serde(default)]
    resolution: Option<u8>,
    #[serde(default)]
    idle_rps: Option<f64>,
    points: Vec<FrontBlob>,
}

#[derive(Deserialize)]
struct FrontBlob {
    sigma_deriv: f64,
    sigma_int: f64,
    wfg: String,
}

/// One point of a fit's front: its weights and what they trade (`sim::margin`).
#[derive(Serialize, Deserialize, Debug)]
pub struct FrontPoint {
    sigma_deriv: f64,
    sigma_int: f64,
    /// How long a roll knock takes to die out, ∫|rate| / knock, ms.
    kick_iae_ms: f64,
    /// Motor roughness the fit's sensor noise causes at hover, V; None without its noise.
    rough_v: Option<f64>,
    /// As [`FlickReport::delay_margin_ms`].
    delay_margin_ms: f64,
    /// As [`FlickReport::trim_deg`].
    trim_deg: f64,
}

/// `fits/<fit>/front/scores.json`.
#[derive(Serialize, Deserialize)]
pub struct FrontReport {
    version: u32,
    fit: String,
    /// The loop period and σ_rate every point's gains were computed for.
    dt: f64,
    sigma_rate: f64,
    /// The [`Grid`] they were scheduled over; None if the front predates recording it.
    #[serde(default)]
    resolution: Option<u8>,
    #[serde(default)]
    idle_rps: Option<f64>,
    /// Whether the fit's gyro filters were modeled; the measures are without them if not.
    gyro_filters: bool,
    max_margin_ms: f64,
    bf_rough_v: Option<f64>,
    points: Vec<FrontPoint>,
}

fn front_index(dir: &Path, fit: &str) -> Result<(PathBuf, FrontIndex), String> {
    let front = dir.join(FITS_DIR).join(plain_name(fit)?).join(FRONT_DIR);
    let index = serde_json::from_slice(&read_file(&front.join(FRONT_INDEX))?)
        .map_err(|e| format!("{FRONT_INDEX}: {e}"))?;
    Ok((front, index))
}

fn score_front(dir: &Path, fit: &str) -> Result<FrontReport, String> {
    use whippyflight_control::sim::margin;
    use whippyunits::value;

    let (front, index) = front_index(dir, fit)?;
    let model = fit_model(dir, fit)?;
    let (noise, bf_rough_v) = fit_noise(dir, fit)?;
    let cutoffs = margin::pt1_cutoffs(&model).ok();
    let gyro_filters = cutoffs.is_some();
    let cutoffs = cutoffs.unwrap_or_default();
    let score = |blob: &FrontBlob| -> Result<FrontPoint, String> {
        let gains = load_blob(&front.join(plain_name(&blob.wfg)?))?;
        Ok(FrontPoint {
            sigma_deriv: blob.sigma_deriv,
            sigma_int: blob.sigma_int,
            kick_iae_ms: value!(margin::kick_iae(&gains, &model, &cutoffs), ms),
            rough_v: noise
                .as_ref()
                .map(|n| value!(margin::hover_roughness(&gains, &model, &cutoffs, n), V)),
            delay_margin_ms: value!(margin::delay_margin(&gains, &model, &cutoffs), ms),
            trim_deg: value!(margin::trim(&gains, &model, &cutoffs).0, deg),
        })
    };
    let workers = thread::available_parallelism().map_or(1, |n| n.get());
    let chunk = index.points.len().div_ceil(workers).max(1);
    let points = thread::scope(|s| {
        let handles: Vec<_> = index
            .points
            .chunks(chunk)
            .map(|blobs| s.spawn(|| blobs.iter().map(score).collect::<Result<Vec<_>, _>>()))
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .map_err(|_| "scoring the front panicked".to_string())?
            })
            .collect::<Result<Vec<_>, _>>()
    })?
    .into_iter()
    .flatten()
    .collect();
    Ok(FrontReport {
        version: 1,
        fit: fit.to_owned(),
        dt: index.dt,
        sigma_rate: index.sigma_rate,
        resolution: index.resolution,
        idle_rps: index.idle_rps,
        gyro_filters,
        max_margin_ms: value!(margin::MAX_MARGIN, ms),
        bf_rough_v,
        points,
    })
}

/// Score the gains front_wfg.py computed for `fit` (the [`Job::Front`]);
/// writes and returns `fits/<fit>/front/scores.json`.
#[tauri::command]
pub async fn whippyflight_front(workdir: String, fit: String) -> Result<FrontReport, String> {
    let dir = self::workdir(&workdir)?;
    tauri::async_runtime::spawn_blocking(move || {
        let report = score_front(&dir, &fit)?;
        let json = serde_json::to_vec(&report).map_err(|e| e.to_string())?;
        let path = dir
            .join(FITS_DIR)
            .join(&fit)
            .join(FRONT_DIR)
            .join(FRONT_SCORES);
        std::fs::write(path, json).map_err(|e| e.to_string())?;
        Ok(report)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Make point `index` of `fit`'s front the gains, as if [`Job::Gen`] had
/// computed them with its weights.
#[tauri::command]
pub fn whippyflight_use_front(workdir: String, fit: String, index: usize) -> Result<(), String> {
    let dir = self::workdir(&workdir)?;
    let (front, front_index) = front_index(&dir, &fit)?;
    let blob = front_index
        .points
        .get(index)
        .ok_or_else(|| format!("no point {index} on the front of {fit}"))?;
    let bytes = read_file(&front.join(plain_name(&blob.wfg)?))?;
    clear_gains_records(&dir);
    std::fs::create_dir_all(dir.join("gains")).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(GAINS), bytes).map_err(|e| e.to_string())?;
    record_gains(
        &dir,
        &fit,
        &Weights {
            sigma_rate: front_index.sigma_rate,
            sigma_deriv: blob.sigma_deriv,
            sigma_int: blob.sigma_int,
        },
    )
}

fn percent_decode(text: &str) -> Result<String, String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3).ok_or("bad percent-encoding")?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| "bad percent-encoding")?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_only_pipeline_files() {
        let dir = Path::new("/w");
        for ok in [
            "flights.json",
            "gains/fit.txt",
            "logs/a.bbl",
            "fits/a_3/diagnostics.json",
            "fits/a_3/coefficients.json",
            "fits/a_3/plots/roll.png",
        ] {
            assert_eq!(resolve(dir, ok).unwrap(), dir.join(ok));
        }
        for bad in [
            "logs/../x",
            "logs/.hidden",
            "logs/a/b",
            "fits/a/other.json",
            "fits/../coefficients.json",
            "fits/a/plots/../../x",
            "fits/a",
            "../etc/passwd",
            "logs/",
            "x",
        ] {
            assert!(resolve(dir, bad).is_err(), "{bad}");
        }
        assert_eq!(
            reveal_path(dir, Some("fits/a/plots")).unwrap(),
            dir.join("fits/a/plots")
        );
        assert!(reveal_path(dir, Some("fits/..")).is_err());
    }

    #[test]
    fn parses_flight_specs() {
        assert_eq!(flight_spec("a.bbl").unwrap(), ("a.bbl", None));
        assert_eq!(flight_spec("a.bbl#3").unwrap(), ("a.bbl", Some(3)));
        for bad in [
            "a.bbl#0",
            "a.bbl#01",
            "a.bbl#+1",
            "a.bbl#",
            "a.bbl#x",
            "../a.bbl#1",
            "#1",
        ] {
            assert!(flight_spec(bad).is_err(), "{bad}");
        }
    }

    const WEIGHTS: Weights = Weights {
        sigma_rate: 5.0,
        sigma_deriv: 10.0,
        sigma_int: 0.012,
    };

    const GRID: Grid = Grid {
        resolution: 3,
        idle_rps: 0.0,
    };

    #[test]
    fn grid_args_are_checked() {
        assert_eq!(
            grid_args(&Grid {
                resolution: 4,
                idle_rps: 130.0
            })
            .unwrap(),
            ["--resolution", "4", "--idle-rps", "1.3e2"]
        );
        for bad in [
            Grid {
                resolution: 1,
                ..GRID
            },
            Grid {
                resolution: MAX_RESOLUTION + 1,
                ..GRID
            },
            Grid {
                idle_rps: -1.0,
                ..GRID
            },
            Grid {
                idle_rps: f64::NAN,
                ..GRID
            },
        ] {
            assert!(grid_args(&bad).is_err(), "{bad:?}");
        }
    }

    /// Runs the real scripts: WHIPPYFLIGHT_PYTHON, WHIPPYFLIGHT_TOOLS and WHIPPYFLIGHT_LOG (a .bbl).
    #[test]
    #[ignore]
    fn fits_and_generates_gains() {
        let env = |name| std::env::var(name).unwrap_or_else(|_| panic!("{name} not set"));
        let toolchain = Toolchain {
            python: env("WHIPPYFLIGHT_PYTHON"),
            tools_dir: env("WHIPPYFLIGHT_TOOLS"),
        };
        let dir = std::env::temp_dir().join(format!("whippyflight-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(LOGS_DIR)).unwrap();
        std::fs::copy(
            env("WHIPPYFLIGHT_LOG"),
            dir.join(LOGS_DIR).join("flight.bbl"),
        )
        .unwrap();
        let jobs = [
            Job::Scan {
                logs: vec!["flight.bbl".into()],
            },
            Job::Fit {
                flights: vec!["flight.bbl#1".into()],
                drag: "auto".into(),
                fit: "flight_1".into(),
            },
            Job::FitPlots {
                fit: "flight_1".into(),
            },
            Job::Noise {
                fit: "flight_1".into(),
                flight: "flight.bbl#1".into(),
            },
            Job::Gen {
                dt: None,
                fit: "flight_1".into(),
                weights: WEIGHTS,
                grid: GRID,
            },
            Job::Front {
                dt: None,
                fit: "flight_1".into(),
                sigma_rate: WEIGHTS.sigma_rate,
                grid: GRID,
            },
        ];
        for job in &jobs {
            let args = job_args(&toolchain, &dir, job).unwrap();
            before_job(&dir, job);
            let status = python_command(&toolchain).args(args).status().unwrap();
            assert!(status.success());
            after_job(&dir, job).unwrap();
        }
        let report = fly_flicks(&dir, None, None).unwrap();
        std::fs::write(dir.join(FLICKS), serde_json::to_vec(&report).unwrap()).unwrap();
        let status = python_command(&toolchain)
            .args(job_args(&toolchain, &dir, &Job::FlickPlots).unwrap())
            .status()
            .unwrap();
        assert!(status.success());
        if let Ok(keep) = std::env::var("WHIPPYFLIGHT_KEEP") {
            let _ = std::fs::remove_dir_all(&keep);
            std::fs::create_dir_all(&keep).unwrap();
            let fit_plots = dir.join(FITS_DIR).join("flight_1").join(PLOTS_DIR);
            for from in [dir.join(FLICK_PLOTS_DIR), fit_plots] {
                for entry in std::fs::read_dir(from).unwrap().flatten() {
                    std::fs::copy(entry.path(), Path::new(&keep).join(entry.file_name())).unwrap();
                }
            }
            std::fs::copy(dir.join(FLICKS), Path::new(&keep).join("flicks.json")).unwrap();
        }
        let ws = whippyflight_workspace(dir.to_string_lossy().into_owned()).unwrap();
        assert!(ws.has_flight_index && ws.has_gains && ws.has_flicks);
        assert_eq!(ws.gains_weights, Some(WEIGHTS));
        assert!(ws.flick_plots.contains(&"flicks.png".to_string()));
        let fit = &ws.fits[0];
        assert!(
            fit.id == "flight_1" && fit.has_coefficients && fit.has_diagnostics && fit.has_noise
        );
        assert!(report.bf_rough_v.is_some());
        assert!(report.flicks.iter().all(|f| f.metrics.rough_v.is_some()));
        assert!(fit.plots.contains(&"roll.png".to_string()));

        let started = std::time::Instant::now();
        let front = score_front(&dir, "flight_1").unwrap();
        eprintln!(
            "{} points scored in {:?}",
            front.points.len(),
            started.elapsed()
        );
        if let Ok(keep) = std::env::var("WHIPPYFLIGHT_KEEP") {
            let json = serde_json::to_vec(&front).unwrap();
            std::fs::write(Path::new(&keep).join("front.json"), json).unwrap();
        }
        assert!(front.gyro_filters && front.points.len() > 1);
        assert_eq!(front.sigma_rate, WEIGHTS.sigma_rate);
        assert_eq!(front.resolution, Some(GRID.resolution));
        assert_eq!(front.idle_rps, Some(GRID.idle_rps));
        let at = |deriv: f64, int: f64| {
            front
                .points
                .iter()
                .position(|p| p.sigma_deriv == deriv && p.sigma_int == int)
                .unwrap()
        };
        let p = |i: usize| &front.points[i];
        for point in &front.points {
            eprintln!("{point:?}");
        }
        // Stiffer rejects a knock faster and is rougher; a firmer integral holds
        // attitude closer and costs delay margin.
        let (stiff, smooth) = (p(at(5.0, 0.012)), p(at(30.0, 0.012)));
        assert!(stiff.kick_iae_ms < smooth.kick_iae_ms);
        assert!(stiff.rough_v.unwrap() > smooth.rough_v.unwrap());
        let (firm, loose) = (p(at(15.0, 0.004)), p(at(15.0, 0.05)));
        assert!(firm.trim_deg < loose.trim_deg);
        assert!(firm.delay_margin_ms < loose.delay_margin_ms);

        // The front's gains at the Gen job's weights are the Gen job's.
        let mine = at(WEIGHTS.sigma_deriv, WEIGHTS.sigma_int);
        let generated = std::fs::read(dir.join(GAINS)).unwrap();
        std::fs::write(dir.join(GAINS), b"stale").unwrap();
        whippyflight_use_front(dir.to_string_lossy().into_owned(), "flight_1".into(), mine)
            .unwrap();
        assert_eq!(std::fs::read(dir.join(GAINS)).unwrap(), generated);
        let ws = whippyflight_workspace(dir.to_string_lossy().into_owned()).unwrap();
        assert_eq!(ws.gains_weights, Some(WEIGHTS));
        assert_eq!(ws.gains_fit.as_deref(), Some("flight_1"));
        assert!(!ws.has_flicks);
        let index = front.points.len();
        assert!(
            whippyflight_use_front(dir.to_string_lossy().into_owned(), "flight_1".into(), index)
                .is_err()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The controller crate's committed test gains, flown on the plant they were computed for.
    #[test]
    fn flies_committed_gains() {
        let tools = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../whippyflight-bf/src/main/flight/whippyflight/tools");
        let dir = std::env::temp_dir().join(format!("whippyflight-flicks-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("fits/meteor")).unwrap();
        std::fs::create_dir_all(dir.join("gains")).unwrap();
        std::fs::copy(
            tools.join("airframes/meteor65.json"),
            dir.join("fits/meteor").join(COEFFICIENTS),
        )
        .unwrap();
        std::fs::copy(tools.join("test_gains.wfg"), dir.join(GAINS)).unwrap();
        std::fs::write(dir.join(GAINS_FIT), "meteor").unwrap();
        std::fs::copy(
            tools.join("airframes/meteor65_noise.bin"),
            dir.join("fits/meteor").join(NOISE),
        )
        .unwrap();
        std::fs::write(
            dir.join("fits/meteor").join(NOISE_INFO),
            r#"{"bf_rough_v": 0.048}"#,
        )
        .unwrap();

        let started = std::time::Instant::now();
        let report = fly_flicks(
            &dir,
            Some(MaxRates {
                rps: [2.0, 1.5, 1.0],
                from: "fc".into(),
            }),
            None,
        )
        .unwrap();
        eprintln!("3 flicks in {:?}", started.elapsed());
        // The panel's stick tracking replaces the logs'.
        let retracked = fly_flicks(&dir, None, Some(1.0)).unwrap();
        assert!(fly_flicks(&dir, None, Some(0.0)).is_err());
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(report.stick_tracking_from, "log");
        assert_ne!(report.stick_tracking, 1.0);
        assert_eq!(retracked.stick_tracking_from, "fc");
        assert_eq!(retracked.stick_tracking, 1.0);
        assert_ne!(retracked.flicks[0].setpoint, report.flicks[0].setpoint);

        assert_eq!(report.fit, "meteor");
        assert!(report.rates_from == "fc" && report.slew > 0.0 && report.jerk > 0.0);
        assert_eq!(report.bf_rough_v, Some(0.048));
        let hover = report.hover_rough_v.expect("flown with noise");
        eprintln!("hover roughness {hover} V");
        assert!(hover > 0.0 && hover < 0.048);
        eprintln!(
            "delay margin {:?} ms, trim {:?}°",
            report.delay_margin_ms, report.trim_deg
        );
        assert!(report.trim_deg.is_some_and(|t| t > 0.0 && t < 45.0));
        assert!(report.delay_margin_ms.is_some_and(|m| m > 0.0));
        assert!(report.trace_dt >= FLICK_TRACE_DT_S * 0.5);
        assert_eq!(
            report.flicks.iter().map(|f| f.amp).collect::<Vec<_>>(),
            [2.0, 1.5, 1.0]
        );
        for f in &report.flicks {
            let n = f.response.len();
            assert!(
                [
                    f.stick.len(),
                    f.setpoint.len(),
                    f.target.len(),
                    f.saturated.len(),
                    f.volts[0].len(),
                    f.volts[3].len()
                ]
                .iter()
                .all(|&m| m == n)
            );
            let m = &f.metrics;
            eprintln!("{} {} rot/s over {:.1} ms: {m:?}", f.axis, f.amp, f.rise_ms);
            // Within the limits the target stays close behind the stick.
            assert!(
                m.target_lag_ms < 0.25 * f.rise_ms && m.target_ovs.abs() < 0.03,
                "{} target: {m:?}",
                f.axis
            );
            assert!(m.rise90_ms.is_some() && m.hold_err.abs() < 0.1);
            let rough = m.rough_v.expect("flown with noise");
            assert!(rough > 0.0 && rough < 1.0, "{} roughness {rough} V", f.axis);
        }
    }

    #[test]
    fn imports_logs_without_clobbering() {
        let dir = std::env::temp_dir().join(format!("whippyflight-import-{}", std::process::id()));
        let src = dir.join("picked");
        std::fs::create_dir_all(src.join("a")).unwrap();
        std::fs::create_dir_all(src.join("b")).unwrap();
        std::fs::write(src.join("a/btfl_001.bbl"), "first drone").unwrap();
        std::fs::write(src.join("b/btfl_001.bbl"), "second drone").unwrap();
        std::fs::write(src.join("dump.txt"), "mixer QUADX").unwrap();
        let work = dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let workdir = work.to_string_lossy().into_owned();
        let picked = |p: &str| src.join(p).to_string_lossy().into_owned();

        let both = vec![picked("a/btfl_001.bbl"), picked("b/btfl_001.bbl")];
        let names = whippyflight_import(workdir.clone(), both.clone(), false).unwrap();
        assert_eq!(names, ["btfl_001.bbl", "btfl_001_2.bbl"]);
        // Importing the same files again finds them where they went.
        assert_eq!(
            whippyflight_import(workdir.clone(), both, false).unwrap(),
            names
        );
        assert_eq!(
            std::fs::read_to_string(work.join("logs/btfl_001_2.bbl")).unwrap(),
            "second drone"
        );

        whippyflight_import(workdir.clone(), vec![picked("dump.txt")], true).unwrap();
        assert_eq!(
            std::fs::read_to_string(work.join(CLI_DUMP)).unwrap(),
            "mixer QUADX"
        );
        assert!(whippyflight_import(workdir, vec![], true).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn decodes_percent_headers() {
        assert_eq!(
            percent_decode("%2FUsers%2Fme%2Fmy%20dir").unwrap(),
            "/Users/me/my dir"
        );
        assert_eq!(percent_decode("logs%2Fa.bbl").unwrap(), "logs/a.bbl");
        assert!(percent_decode("%zz").is_err());
    }
}
