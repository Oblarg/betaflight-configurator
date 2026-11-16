//! WhippyFlight tuning pipeline for the WhippyFlight tab.
//!
//! The regression, noise and log tools (`whippyflight_betaflight.*`) and the
//! gain computation (`whippyflight.gen_wfg`) are modules of the whippyflight
//! packages installed in the user-chosen interpreter. The webview never supplies a command line: it names a job and
//! its typed parameters, and the arguments are built here. Every file the
//! pipeline reads or writes sits at a fixed path inside one working directory:
//!
//! ```text
//! <workdir>/logs/*.bbl          flight logs pulled from the FC
//! <workdir>/flights.json        the flights in each log (`fit_model flights`)
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
use whippyflight_sim::units::{AngAccel, Freq, Millis, Rate, Time, Voltage};
use whippyunits::api::rescale;
use whippyunits::{qty, quantity, value};

type AngJerk = qty!(rot / s ^ 3);
type Degrees = qty!(deg);
/// Traces are sent as f32.
type RateF32 = qty!(rot / s, f32);
type VoltageF32 = qty!(V, f32);

/// Quantities in JSON as plain numbers (`#[serde(with = "plain")]`), each in
/// its field type's own unit, so the webview and the Python tools read and
/// write bare numbers.
mod plain {
    use serde::de::{DeserializeOwned, Error};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use whippyunits::Quantity;

    pub trait Plain: Sized {
        type Raw: Serialize + DeserializeOwned;
        fn to_raw(&self) -> Self::Raw;
        fn from_raw(raw: Self::Raw) -> Result<Self, String>;
    }

    impl<U: Clone, T: Copy + Serialize + DeserializeOwned, B: Clone> Plain for Quantity<U, T, B> {
        type Raw = T;
        fn to_raw(&self) -> T {
            self.unsafe_value
        }
        fn from_raw(raw: T) -> Result<Self, String> {
            Ok(*Self::from_ref(&raw))
        }
    }

    impl<P: Plain> Plain for Option<P> {
        type Raw = Option<P::Raw>;
        fn to_raw(&self) -> Self::Raw {
            self.as_ref().map(P::to_raw)
        }
        fn from_raw(raw: Self::Raw) -> Result<Self, String> {
            raw.map(P::from_raw).transpose()
        }
    }

    impl<P: Plain> Plain for Vec<P> {
        type Raw = Vec<P::Raw>;
        fn to_raw(&self) -> Self::Raw {
            self.iter().map(P::to_raw).collect()
        }
        fn from_raw(raw: Self::Raw) -> Result<Self, String> {
            raw.into_iter().map(P::from_raw).collect()
        }
    }

    impl<P: Plain, const N: usize> Plain for [P; N] {
        type Raw = Vec<P::Raw>;
        fn to_raw(&self) -> Self::Raw {
            self.iter().map(P::to_raw).collect()
        }
        fn from_raw(raw: Self::Raw) -> Result<Self, String> {
            let len = raw.len();
            let items = raw.into_iter().map(P::from_raw).collect::<Result<Vec<_>, _>>()?;
            items
                .try_into()
                .map_err(|_| format!("expected {N} values, not {len}"))
        }
    }

    pub fn serialize<P: Plain, S: Serializer>(value: &P, serializer: S) -> Result<S::Ok, S::Error> {
        value.to_raw().serialize(serializer)
    }

    pub fn deserialize<'de, P: Plain, D: Deserializer<'de>>(deserializer: D) -> Result<P, D::Error> {
        P::from_raw(P::Raw::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// The whippyflight packages' tools, run as modules of the toolchain's Python: the
/// gain tools are firmware-neutral, the fit, noise and log tools Betaflight's.
const FIT_MODULE: &str = "whippyflight_betaflight.fit_model";
const GEN_MODULE: &str = "whippyflight.gen_wfg";
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
const FLICK_PLOT_MODULE: &str = "whippyflight.plot_flicks";
const FIT_PLOT_MODULE: &str = "whippyflight_betaflight.fit_diagnostics";
const NOISE_MODULE: &str = "whippyflight_betaflight.extract_noise";
/// A fit's sensor noise (extract_noise), and its window and BF's roughness there.
const NOISE: &str = "noise.bin";
const NOISE_INFO: &str = "noise.json";
const FRONT_MODULE: &str = "whippyflight.front_wfg";
/// A fit's Pareto front: the blobs front_wfg.py computed, their index, and their scores.
const FRONT_DIR: &str = "front";
const FRONT_INDEX: &str = "front.json";
const FRONT_SCORES: &str = "scores.json";
/// Flick traces are kept at this spacing (or the loop period, if longer).
const FLICK_TRACE_DT: Time = quantity!(5e-4, s);
const REQUIRED_MODULES: &[&str] = &[
    "whippyflight",
    "whippyflight_betaflight",
    "numpy",
    "scipy",
    "control",
    "frccontrol",
];

#[derive(Default)]
pub struct WhippyFlightState {
    child: Arc<Mutex<Option<Child>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Toolchain {
    python: String,
}

/// gen_wfg.py's LQR cost weights (Bryson: the excursion each state may make).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Weights {
    /// Rate excursion.
    #[serde(with = "plain")]
    sigma_rate: Rate,
    /// Derivative-state excursion.
    #[serde(with = "plain")]
    sigma_deriv: AngAccel,
    /// Low-passed rate error the integral holds.
    #[serde(with = "plain")]
    sigma_int: Rate,
}

/// The motor-speed grid gen_wfg.py schedules the gains over.
#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Grid {
    /// Speeds per motor, N: the blob holds N⁴ points.
    resolution: u8,
    /// The FC's `whippyflight_idle_rps`; the grid starts there when it is above
    /// gen_wfg's minimum. 0 for none.
    #[serde(with = "plain")]
    idle_rps: Rate,
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
        /// Loop period: the FC's; the fit's logged loop rate if None.
        #[serde(default, with = "plain")]
        dt: Option<Time>,
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
        #[serde(default, with = "plain")]
        dt: Option<Time>,
        fit: String,
        #[serde(with = "plain")]
        sigma_rate: Rate,
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
    /// Its sensor noise's RMS as logged; None without it.
    noise_rms: Option<NoiseRms>,
    /// The turbulence its diagnostics measured; None without it.
    turbulence: Option<TurbulenceSpec>,
    /// The latency it measured; None for a fit without it (flown as none).
    latency: Option<Latency>,
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
        .env("PYTHONUNBUFFERED", "1")
        .env("MPLBACKEND", "Agg")
        .stdin(Stdio::null());
    command
}

/// The interpreter arguments that run `module` (`python -m`).
fn module(name: &str) -> Vec<String> {
    vec!["-m".into(), name.into()]
}

/// The `--dt` argument, if the loop period is given.
fn dt_args(dt: Option<Time>) -> Result<Vec<String>, String> {
    let Some(dt) = dt else {
        return Ok(vec![]);
    };
    if !(quantity!(1e-5, s)..quantity!(1e-2, s)).contains(&dt) {
        return Err(format!("implausible PID loop period: {} s", value!(dt, s)));
    }
    Ok(vec!["--dt".into(), format!("{:e}", value!(dt, s))])
}

/// A σ argument: `value`, in the unit gen_wfg.py takes it in.
fn sigma_args(flag: &str, value: f64) -> Result<[String; 2], String> {
    if value.is_finite() && value > 0.0 {
        Ok([flag.into(), format!("{value:e}")])
    } else {
        Err(format!("{flag} must be positive, not {value}"))
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
    let idle_rps = value!(grid.idle_rps, rot / s);
    if !(idle_rps.is_finite() && idle_rps >= 0.0) {
        return Err(format!("implausible idle speed: {idle_rps} rev/s"));
    }
    Ok(vec![
        "--resolution".into(),
        grid.resolution.to_string(),
        "--idle-rps".into(),
        format!("{idle_rps:e}"),
    ])
}

fn job_args(dir: &Path, job: &Job) -> Result<Vec<String>, String> {
    let text = |path: PathBuf| path.to_string_lossy().into_owned();
    let log = |name: &str| -> Result<String, String> {
        Ok(text(dir.join(LOGS_DIR).join(plain_name(name)?)))
    };
    match job {
        Job::Scan { logs } => {
            if logs.is_empty() {
                return Err("no logs to scan".into());
            }
            let mut args = module(FIT_MODULE);
            args.push("flights".into());
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
            let mut args = module(FIT_MODULE);
            args.push("fit".into());
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
            let dt = dt_args(*dt)?;
            let sigmas = [
                sigma_args("--sigma-rate", value!(weights.sigma_rate, rot / s))?,
                sigma_args("--sigma-deriv", value!(weights.sigma_deriv, rot / s ^ 2))?,
                sigma_args("--sigma-int", value!(weights.sigma_int, rot / s))?,
            ];
            let grid = grid_args(grid)?;
            let coefficients = dir.join(FITS_DIR).join(plain_name(fit)?).join(COEFFICIENTS);
            if !coefficients.is_file() {
                return Err(format!("no fit {fit} to compute gains from"));
            }
            std::fs::create_dir_all(dir.join("gains")).map_err(|e| e.to_string())?;
            let mut args = module(GEN_MODULE);
            args.extend([text(coefficients), text(dir.join(GAINS))]);
            args.extend(dt);
            args.extend(sigmas.into_iter().flatten());
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
            let mut args = module(NOISE_MODULE);
            args.extend([
                index.map_or(path.clone(), |n| format!("{path}#{n}")),
                text(out.join(NOISE)),
                "--json".into(),
                text(out.join(NOISE_INFO)),
            ]);
            // The fit's model takes the motion it explains out, its diagnostics place the splits.
            for (flag, file) in [("--fit", COEFFICIENTS), ("--diagnostics", DIAGNOSTICS)] {
                if out.join(file).is_file() {
                    args.extend([flag.into(), text(out.join(file))]);
                }
            }
            Ok(args)
        }
        Job::FitPlots { fit } => {
            let out = dir.join(FITS_DIR).join(plain_name(fit)?);
            if !out.join(DIAGNOSTICS).is_file() {
                return Err(format!("no diagnostics for fit {fit} to plot"));
            }
            let _ = std::fs::remove_dir_all(out.join(PLOTS_DIR));
            let mut args = module(FIT_PLOT_MODULE);
            args.extend([text(out.join(DIAGNOSTICS)), text(out.join(PLOTS_DIR))]);
            Ok(args)
        }
        Job::Front {
            dt,
            fit,
            sigma_rate,
            grid,
        } => {
            let dt = dt_args(*dt)?;
            let sigma_rate = sigma_args("--sigma-rate", value!(*sigma_rate, rot / s))?;
            let grid = grid_args(grid)?;
            let out = dir.join(FITS_DIR).join(plain_name(fit)?);
            if !out.join(COEFFICIENTS).is_file() {
                return Err(format!("no fit {fit} to compute gains from"));
            }
            let _ = std::fs::remove_dir_all(out.join(FRONT_DIR));
            let mut args = module(FRONT_MODULE);
            args.extend([text(out.join(COEFFICIENTS)), text(out.join(FRONT_DIR))]);
            args.extend(sigma_rate);
            args.extend(dt);
            args.extend(grid);
            Ok(args)
        }
        Job::FlickPlots => {
            if !dir.join(FLICKS).is_file() {
                return Err("no flicks to plot".into());
            }
            let _ = std::fs::remove_dir_all(dir.join(FLICK_PLOTS_DIR));
            let mut args = module(FLICK_PLOT_MODULE);
            args.extend([text(dir.join(FLICKS)), text(dir.join(FLICK_PLOTS_DIR))]);
            Ok(args)
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

/// Python version and missing modules for a toolchain.
#[tauri::command]
pub async fn whippyflight_check_env(toolchain: Toolchain) -> Result<EnvReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
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
    let args = job_args(&dir, &job)?;
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
                has_noise: path.join(NOISE).is_file() && noise_current(&path),
                noise_rms: load_noise(&path)
                    .ok()
                    .and_then(|(noise, _)| noise)
                    .map(|n| NoiseRms::of(&n, 1.0, 1.0)),
                turbulence: fit_turbulence(&path),
                latency: fit_latency(&path),
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

/// One flick (see `whippyflight_sim::flick`): ratios are of `amp`;
/// `rise90_ms` is null when 90% was never reached.
#[derive(Serialize, Deserialize, Debug)]
pub struct FlickMetrics {
    target_ovs: f64,
    #[serde(with = "plain")]
    target_lag_ms: Millis,
    #[serde(with = "plain")]
    gyro_lag_ms: Millis,
    #[serde(with = "plain")]
    rise90_ms: Option<Millis>,
    overshoot: f64,
    hold_err: f64,
    track_rms: f64,
    residual: f64,
    sat_frac: f64,
    /// Motor roughness from the fit's sensor noise; None without its noise.
    #[serde(with = "plain")]
    rough_v: Option<Voltage>,
}

/// One flick's traces from the stick's start, every `trace_dt`: `response`
/// is the gyro with the stick minus without it.
#[derive(Serialize, Deserialize)]
pub struct FlickResult {
    /// "roll", "pitch" or "yaw".
    axis: String,
    #[serde(with = "plain")]
    rise_ms: Millis,
    /// The fastest rise the target limits allow; `rise_ms` is longer when a
    /// slower stick was asked for.
    #[serde(with = "plain")]
    limit_rise_ms: Millis,
    /// FRD: negative for a flick the other way.
    #[serde(with = "plain")]
    amp: Rate,
    metrics: FlickMetrics,
    #[serde(with = "plain")]
    stick: Vec<RateF32>,
    #[serde(with = "plain")]
    setpoint: Vec<RateF32>,
    #[serde(with = "plain")]
    target: Vec<RateF32>,
    #[serde(with = "plain")]
    response: Vec<RateF32>,
    /// Ticks within each trace sample the motors saturated, as a fraction.
    saturated: Vec<f32>,
    /// Commanded voltage of each motor (noise-free).
    #[serde(with = "plain")]
    volts: [Vec<VoltageF32>; 4],
    /// Each motor's shaft speed, by firmware index (the plant's, without telemetry noise).
    #[serde(with = "plain")]
    motor_speeds: [Vec<RateF32>; 4],
}

/// `gains/flicks.json`: a full-stick flick each way on each axis, the fastest
/// raised cosine within the gains' target slew and jerk limits (or slower, to
/// `min_rise_ms`).
#[derive(Serialize, Deserialize)]
pub struct FlickReport {
    version: u32,
    /// The fit whose plant the gains were flown on.
    fit: String,
    /// The slowest stick asked for; None for the fastest the limits allow.
    #[serde(with = "plain")]
    min_rise_ms: Option<Millis>,
    /// The feedback gains flown; those off were zeroed.  The margins, trim and
    /// hover roughness are still the full gains'.
    feedback: FlickFeedback,
    /// What they were asked to fly in.
    conditions: FlickConditions,
    /// The delays flown (the fit's, or the conditions'), in whole controller ticks.
    latency: Latency,
    /// The sensor noise they flew in; None without the fit's noise.
    noise_rms: Option<NoiseRms>,
    /// The turbulence they flew in; None for none.
    turbulence: Option<TurbulenceSpec>,
    /// Whether the gyro went through the fit's filters: asked, and modeled.
    gyro_filtered: bool,
    #[serde(with = "plain")]
    loop_dt: Time,
    #[serde(with = "plain")]
    trace_dt: Time,
    /// How long the stick holds before releasing, after it gets there.
    #[serde(with = "plain")]
    hold_s: Time,
    /// The target limits.
    #[serde(with = "plain")]
    slew: AngAccel,
    #[serde(with = "plain")]
    jerk: AngJerk,
    /// Where the amplitudes came from: "fc", "log" (the fit's blackbox header),
    /// "manual" (set in the tab) or "default" (Betaflight's).
    rates_from: String,
    /// The stick estimator's tracking index flown (`whippyflight_stick_tracking`).
    stick_tracking: f32,
    /// Where it came from: "fc" (the settings panel) or "log" (the fit's blackbox header).
    stick_tracking_from: String,
    /// The motor-speed low-pass flown (`whippyflight_rpm_lpf_hz`); 0 for off.
    #[serde(with = "plain")]
    motor_lpf_hz: Freq,
    /// Where it came from, as `stick_tracking_from`.
    motor_lpf_from: String,
    /// BF's own motor roughness over the window of the log the noise came from:
    /// everything above 50 Hz in its motor outputs at hover, not only what noise caused.
    #[serde(with = "plain")]
    bf_rough_v: Option<Voltage>,
    /// What the fit's sensor noise adds to these gains' motor voltages at hover
    /// (`sim::margin::hover_roughness`), the like-for-like figure beside
    /// [`bf_rough_v`]; None without the noise.
    #[serde(with = "plain")]
    hover_rough_v: Option<Voltage>,
    /// Extra gyro delay the gains tolerate on top of the fit's gyro filters and
    /// [`latency`] (`sim::margin`); −1 ms if they don't fly cleanly even without,
    /// None if the filters aren't modeled.
    #[serde(with = "plain")]
    delay_margin_ms: Option<Millis>,
    /// The most [`delay_margin_ms`] reports.
    #[serde(with = "plain")]
    max_margin_ms: Millis,
    /// Attitude a 5% CG shift costs before the integral takes up the
    /// torque (`sim::margin::trim`); None if the filters aren't modeled.
    #[serde(with = "plain")]
    trim_deg: Option<Degrees>,
    flicks: Vec<FlickResult>,
}

/// Which feedback gains the flicks fly with (`whippyflight_control::gains::FeedbackTerms`);
/// all off for the feedforward alone.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct FlickFeedback {
    p: bool,
    i: bool,
    d: bool,
}

impl Default for FlickFeedback {
    fn default() -> Self {
        Self { p: true, i: true, d: true }
    }
}

impl From<FlickFeedback> for whippyflight_control::gains::FeedbackTerms {
    fn from(f: FlickFeedback) -> Self {
        Self { p: f.p, i: f.i, d: f.d }
    }
}

/// What the flicks fly in besides the stick (`whippyflight_sim::flick::Conditions`).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct FlickConditions {
    /// The fit's gyro low-pass filters between the body rates and the controller.
    gyro_filter: bool,
    /// The fit's recorded gyro and motor-speed noise, each on or off.
    gyro_noise: bool,
    motor_noise: bool,
    /// RMS of the gyro noise over its axes and of the motor-speed noise over the
    /// motors (the fit's recording, rescaled); None for as logged.
    #[serde(default, with = "plain")]
    gyro_noise_rms: Option<Rate>,
    #[serde(default, with = "plain")]
    motor_speed_noise_rms: Option<Rate>,
    /// Fly the plotted flights with the noise; otherwise it only sets the roughness.
    noisy_flights: bool,
    /// CG offset [right, forward], % of the arm.
    cg_shift_pct: [f64; 2],
    turbulence: bool,
    /// Turbulence: RMS angular acceleration per axis [roll, pitch, yaw], and its
    /// bandwidth; None for the fit's (or, without it, none and 10 Hz).
    #[serde(default, with = "plain")]
    turbulence_rms: Option<[AngAccel; 3]>,
    #[serde(default, with = "plain")]
    turbulence_hz: Option<Freq>,
    /// Motor-command and speed-telemetry delays; None for the fit's.  They set
    /// the plant, so the margins and trim fly them too.
    #[serde(default, with = "plain")]
    actuator_delay_ms: Option<Millis>,
    #[serde(default, with = "plain")]
    telemetry_delay_ms: Option<Millis>,
}

impl Default for FlickConditions {
    fn default() -> Self {
        Self {
            gyro_filter: true,
            gyro_noise: true,
            motor_noise: true,
            gyro_noise_rms: None,
            motor_speed_noise_rms: None,
            noisy_flights: false,
            cg_shift_pct: [0.0; 2],
            turbulence: false,
            turbulence_rms: None,
            turbulence_hz: None,
            actuator_delay_ms: None,
            telemetry_delay_ms: None,
        }
    }
}

impl FlickConditions {
    fn check(&self) -> Result<(), String> {
        let gyro = self.gyro_noise_rms.map(|g| value!(g, rot / s));
        let motor = self.motor_speed_noise_rms.map(|m| value!(m, rot / s));
        if ![gyro, motor].iter().flatten().all(|x| x.is_finite() && *x >= 0.0) {
            return Err(format!("bad noise RMS {gyro:?}, {motor:?} rot/s"));
        }
        if !self.cg_shift_pct.iter().all(|x| x.is_finite() && x.abs() <= 50.0) {
            return Err(format!("CG offsets must be within ±50%, not {:?}", self.cg_shift_pct));
        }
        let rms = self.turbulence_rms.map(|r| r.map(|a| value!(a, rot / s ^ 2)));
        let hz = self.turbulence_hz.map(|f| value!(f, Hz));
        if !rms.iter().flatten().all(|x| x.is_finite() && *x >= 0.0)
            || !hz.iter().all(|x| x.is_finite() && *x > 0.0 && *x <= 1000.0)
        {
            return Err(format!("bad turbulence {rms:?} rot/s² at {hz:?} Hz"));
        }
        let delays = [self.actuator_delay_ms, self.telemetry_delay_ms];
        if !delays.iter().flatten().all(|&d| value!(d, ms).is_finite() && quantity!(0.0, ms) <= d && d <= MAX_FLICK_DELAY) {
            return Err(format!(
                "delays must be 0–{} ms, not {:?} ms",
                value!(MAX_FLICK_DELAY, ms),
                delays.map(|d| d.map(|d| value!(d, ms)))
            ));
        }
        Ok(())
    }
}

/// RMS of a sensor noise recording, scaled: the gyro over its axes, the motor
/// speed over the motors at the recording's motor speed.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct NoiseRms {
    #[serde(with = "plain")]
    gyro: Rate,
    #[serde(with = "plain")]
    motor: Rate,
    /// How the motor-speed noise grows with each motor's speed: as speed^this;
    /// None for the recording replayed alike at every speed.
    motor_speed_exponent: Option<f64>,
}

impl NoiseRms {
    fn of(noise: &whippyflight_sim::noise::Noise, gyro_scale: f32, motor_scale: f32) -> Self {
        let (gyro, motor) = noise.rms();
        Self {
            gyro: gyro * gyro_scale as f64,
            motor: motor * motor_scale as f64,
            motor_speed_exponent: noise.speed_law().map(|law| law.exponent),
        }
    }
}

/// Turbulence: RMS angular acceleration per axis [roll, pitch, yaw], and one bandwidth.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct TurbulenceSpec {
    #[serde(with = "plain")]
    rms: [AngAccel; 3],
    #[serde(with = "plain")]
    hz: Freq,
}

/// What `fits/<fit>/diagnostics.json` measured of the turbulence: each axis's
/// RMS, and their corners averaged weighted by power.  None unless every axis has one.
fn fit_turbulence(fit_dir: &Path) -> Option<TurbulenceSpec> {
    let diag: serde_json::Value = serde_json::from_slice(&std::fs::read(fit_dir.join(DIAGNOSTICS)).ok()?).ok()?;
    let axes = ["roll", "pitch", "yaw"].map(|a| {
        let t = &diag["turbulence"][a];
        Some((quantity!(t["rms"].as_f64()?, rot / s ^ 2), quantity!(t["hz"].as_f64()?, Hz)))
    });
    let axes: [(AngAccel, Freq); 3] = [axes[0]?, axes[1]?, axes[2]?];
    let power = axes.map(|(rms, _)| rms * rms);
    let total = power.iter().fold(quantity!(0.0, rot ^ 2 / s ^ 4), |sum, &p| sum + p);
    if total <= quantity!(0.0, rot ^ 2 / s ^ 4) {
        return None;
    }
    let hz = axes
        .iter()
        .zip(power)
        .fold(quantity!(0.0, Hz), |sum, (&(_, hz), p)| sum + hz * f64::from(p / total));
    Some(TurbulenceSpec { rms: axes.map(|(rms, _)| rescale(rms)), hz })
}

/// Dead time between a motor command and the motors acting on it, and between
/// their speed and its report to the controller (`whippyflight_sim::plant::Model`).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct Latency {
    #[serde(with = "plain")]
    actuator_ms: Millis,
    #[serde(with = "plain")]
    telemetry_ms: Millis,
}

/// The longest delay the flicks can be asked to fly.
const MAX_FLICK_DELAY: Millis = quantity!(20.0, ms);

/// What `fits/<fit>/coefficients.json` measured of the latency; None for a fit without it.
fn fit_latency(fit_dir: &Path) -> Option<Latency> {
    let c: serde_json::Value = serde_json::from_slice(&std::fs::read(fit_dir.join(COEFFICIENTS)).ok()?).ok()?;
    let ms = |key: &str| c["latency"][key].as_f64().map(|v| quantity!(v, ms));
    Some(Latency { actuator_ms: ms("actuator_ms")?, telemetry_ms: ms("telemetry_ms")? })
}

/// The slowest stick rise the flicks can be asked for.
const MAX_MIN_RISE: Millis = quantity!(1000.0, ms);

/// Betaflight's default max rate, on every axis.
fn default_max_rate() -> Rate {
    rescale(quantity!(670.0, deg / s))
}

/// The rise of the fastest raised-cosine stick to `amp` whose peak slope
/// Aπ/2T and slope change Aπ²/2T² stay within `slew` and `jerk`.
fn limit_rise(amp: Rate, slew: AngAccel, jerk: AngJerk) -> Time {
    use std::f64::consts::PI;
    let by_jerk: Time = PI * (amp / (2.0 * jerk)).sqrt();
    let by_slew: Time = PI * amp / (2.0 * slew);
    by_jerk.max(by_slew)
}

/// Full-stick roll, pitch and yaw rates, and where they came from.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaxRates {
    #[serde(with = "plain")]
    rps: [Rate; 3],
    /// "fc", "log" or "manual" (set in the tab).
    from: String,
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The plant `fits/<fit>` fitted.
fn fit_model(dir: &Path, fit: &str) -> Result<whippyflight_sim::plant::Model, String> {
    let coefficients = dir.join(FITS_DIR).join(plain_name(fit)?).join(COEFFICIENTS);
    let text = String::from_utf8(read_file(&coefficients)?).map_err(|e| e.to_string())?;
    whippyflight_sim::plant::Model::from_json(&text, &coefficients.to_string_lossy())
}

/// A fit's sensor noise, if it has been extracted, and BF's own roughness over it.
fn fit_noise(
    dir: &Path,
    fit: &str,
) -> Result<(Option<whippyflight_sim::noise::Noise>, Option<Voltage>), String> {
    load_noise(&dir.join(FITS_DIR).join(plain_name(fit)?))
}

/// Whether the fit in `fit_dir`'s `noise.json` is from an extractor that
/// measures the motor-speed noise's speed law (null where it couldn't);
/// an older extraction is redone.
fn noise_current(fit_dir: &Path) -> bool {
    std::fs::read(fit_dir.join(NOISE_INFO))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_some_and(|info| info.get("motor_noise_speed").is_some())
}

/// [`fit_noise`] of the fit in `fit_dir`: the recording, its motor-speed
/// noise following the speed law `noise.json` measured (if any).
fn load_noise(fit_dir: &Path) -> Result<(Option<whippyflight_sim::noise::Noise>, Option<Voltage>), String> {
    use whippyflight_sim::noise::{Noise, SpeedLaw};
    let info = std::fs::read(fit_dir.join(NOISE_INFO))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .unwrap_or_default();
    let law = &info["motor_noise_speed"];
    let law = law["reference_rps"]
        .as_f64()
        .zip(law["exponent"].as_f64())
        .map(|(reference, exponent)| SpeedLaw { reference: quantity!(reference, rot / s), exponent });
    let noise = match std::fs::read(fit_dir.join(NOISE)) {
        Ok(bytes) => Some(Noise::from_bytes(&bytes, quantity!(0.0, s), 1.0, 1.0)?.with_speed_law(law)?),
        Err(_) => None,
    };
    let bf_rough_v = info["bf_rough_v"].as_f64().map(|v| quantity!(v, V));
    Ok((noise, bf_rough_v))
}

fn load_blob(path: &Path) -> Result<whippyflight_control::gains::GainStorage, String> {
    load_blob_bytes(path, &read_file(path)?)
}

/// [`load_blob`] of `bytes`, read from `path`.
fn load_blob_bytes(path: &Path, bytes: &[u8]) -> Result<whippyflight_control::gains::GainStorage, String> {
    use whippyflight_control::gains::{GainStorage, load_gains};
    use whippyflight_sim::static_blob;
    use whippyflight_control::types::GainLoadResult;
    let mut gains = GainStorage::new();
    // load_gains reads the tables in place for as long as the gains are used; the blob is
    // a few kB, leaked once per simulation.
    let result = load_gains(static_blob(bytes), &mut gains);
    if result != GainLoadResult::Ok {
        return Err(format!("{} doesn't load: {result:?}", path.display()));
    }
    Ok(gains)
}

fn fly_flicks(
    dir: &Path,
    max_rates: Option<MaxRates>,
    stick_tracking: Option<f32>,
    motor_lpf: Option<Freq>,
    min_rise: Option<Millis>,
    feedback: FlickFeedback,
    conditions: FlickConditions,
) -> Result<FlickReport, String> {
    conditions.check()?;
    use whippyflight_control::setpoint::target_limits;
    use whippyflight_sim::{flick, margin};

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
    let motor_lpf_from = match motor_lpf {
        Some(hz) if value!(hz, Hz).is_finite() && hz >= quantity!(0.0, Hz) => {
            model.motor_lpf = hz;
            "fc"
        }
        Some(hz) => return Err(format!("bad motor speed low-pass {} Hz", value!(hz, Hz))),
        None => "log",
    };
    if let Some(d) = conditions.actuator_delay_ms {
        model.actuator_delay = rescale(d);
    }
    if let Some(d) = conditions.telemetry_delay_ms {
        model.telemetry_delay = rescale(d);
    }
    let blob = read_file(&dir.join(GAINS))?;
    let gains = load_blob_bytes(&dir.join(GAINS), &blob)?;
    let partial = match feedback == FlickFeedback::default() {
        true => None,
        false => {
            let blob = whippyflight_control::gains::with_feedback(&blob, feedback.into());
            Some(load_blob_bytes(&dir.join(GAINS), &blob)?)
        }
    };
    let flown = partial.as_ref().unwrap_or(&gains);
    let loop_dt: Time = gains.header.dt.lossy_into();
    let stride = (f64::from(FLICK_TRACE_DT / loop_dt).round() as usize).max(1);
    let thin = |v: &[Rate]| -> Vec<RateF32> {
        v.iter().step_by(stride).map(|r| r.lossy_into()).collect()
    };
    let (rates, rates_from) = match max_rates {
        Some(MaxRates { rps, from }) if ["fc", "log", "manual"].contains(&from.as_str()) => (rps, from),
        Some(MaxRates { from, .. }) => return Err(format!("unknown rate source {from}")),
        None => ([default_max_rate(); 3], "default".into()),
    };
    let rates_rps = rates.map(|r| value!(r, rot / s));
    if rates_rps.iter().any(|r| !(r.is_finite() && *r > 0.0)) {
        return Err(format!("bad max rates {rates_rps:?} rot/s"));
    }
    if let Some(rise) = min_rise {
        if !(quantity!(0.0, ms) < rise && rise <= MAX_MIN_RISE) {
            return Err(format!("bad stick rise {} ms", value!(rise, ms)));
        }
    }
    let (noise, bf_rough_v) = fit_noise(dir, &fit)?;
    let cutoffs = margin::pt1_cutoffs(&model).ok();
    let delay_margin_ms = cutoffs
        .as_ref()
        .map(|c| margin::delay_margin(&gains, &model, c));
    let trim_deg = cutoffs
        .as_ref()
        .map(|c| rescale(margin::trim(&gains, &model, c).0));
    let hover_rough_v = noise.as_ref().map(|n| {
        let filters = cutoffs.as_deref().unwrap_or_default();
        margin::hover_roughness(&gains, &model, filters, n)
    });
    let gyro_filtered = conditions.gyro_filter && cutoffs.is_some();
    let (gyro_scale, motor_scale) = noise.as_ref().map_or((0.0, 0.0), |n| {
        let (gyro, motor) = n.rms();
        let scale = |on: bool, asked: Option<Rate>, logged: Rate| match asked {
            _ if !on => 0.0,
            None => 1.0,
            Some(_) if logged <= quantity!(0.0, rot / s) => 0.0,
            Some(asked) => f64::from(asked / logged) as f32,
        };
        (
            scale(conditions.gyro_noise, conditions.gyro_noise_rms, gyro),
            scale(conditions.motor_noise, conditions.motor_speed_noise_rms, motor),
        )
    });
    let flick_noise = noise
        .as_ref()
        .filter(|_| gyro_scale > 0.0 || motor_scale > 0.0)
        .map(|n| n.scaled(gyro_scale, motor_scale));
    let logged_turbulence = fit_turbulence(&dir.join(FITS_DIR).join(plain_name(&fit)?));
    let turbulence = conditions.turbulence.then(|| TurbulenceSpec {
        rms: conditions
            .turbulence_rms
            .or(logged_turbulence.map(|t| t.rms))
            .unwrap_or([quantity!(0.0, rot / s ^ 2); 3]),
        hz: conditions
            .turbulence_hz
            .or(logged_turbulence.map(|t| t.hz))
            .unwrap_or(quantity!(10.0, Hz)),
    });
    let turbulence = turbulence.filter(|t| t.rms.iter().any(|&a| a > quantity!(0.0, rot / s ^ 2)));
    let flick_conditions = flick::Conditions {
        gyro_filter: if gyro_filtered { cutoffs.as_deref().unwrap_or_default() } else { &[] },
        noise: flick_noise.as_ref(),
        noisy_flights: conditions.noisy_flights,
        cg_shift: conditions.cg_shift_pct.map(|p| p / 100.0),
        turbulence: turbulence.map(|t| flick::Turbulence {
            rms: t.rms,
            bandwidth: t.hz,
            seed: 1,
        }),
    };
    let (slew, jerk) = target_limits(&gains);
    let (slew, jerk): (AngAccel, AngJerk) = (slew.lossy_into(), jerk.lossy_into());
    let flicks = (0..3)
        .flat_map(|axis| [(axis, 1.0), (axis, -1.0)])
        .map(|(axis, direction)| {
            let amp = direction * rates[axis];
            let fastest = limit_rise(rates[axis], slew, jerk);
            let rise = min_rise.map_or(fastest, |min| fastest.max(rescale(min)));
            let f = flick::measure(
                flown,
                &model,
                axis,
                amp,
                rise,
                flick::Limits::default(),
                &flick_conditions,
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
                rise_ms: rescale(rise),
                limit_rise_ms: rescale(fastest),
                amp,
                metrics: FlickMetrics {
                    target_ovs: m.target_ovs,
                    target_lag_ms: rescale(m.target_lag),
                    gyro_lag_ms: rescale(m.gyro_lag),
                    rise90_ms: Some(m.rise90)
                        .filter(|t| value!(*t, s).is_finite())
                        .map(rescale),
                    overshoot: m.overshoot,
                    hold_err: m.hold_err,
                    track_rms: m.track_rms,
                    residual: m.residual,
                    sat_frac: m.sat_frac,
                    rough_v: m.rough_v,
                },
                stick: thin(&f.run.stick),
                setpoint: thin(&f.run.setpoint),
                target: thin(&f.run.target),
                response: thin(&f.response),
                saturated,
                volts: std::array::from_fn(|m| {
                    f.run.volts.iter().step_by(stride).map(|v| v[m].lossy_into()).collect()
                }),
                motor_speeds: std::array::from_fn(|m| {
                    f.run.motor_speeds.iter().step_by(stride).map(|w| w[m].lossy_into()).collect()
                }),
            }
        })
        .collect();
    let flown = |delay: Time| rescale(whippyflight_sim::plant::DelayLine::<()>::flown(delay, loop_dt));
    Ok(FlickReport {
        version: 18,
        fit,
        min_rise_ms: min_rise,
        feedback,
        conditions,
        latency: Latency {
            actuator_ms: flown(model.actuator_delay),
            telemetry_ms: flown(model.telemetry_delay),
        },
        noise_rms: noise.as_ref().map(|n| NoiseRms::of(n, gyro_scale, motor_scale)),
        turbulence,
        gyro_filtered,
        loop_dt,
        trace_dt: loop_dt * stride as f64,
        hold_s: flick::HOLD,
        slew,
        jerk,
        rates_from,
        stick_tracking: model.stick_tracking,
        stick_tracking_from: stick_tracking_from.into(),
        motor_lpf_hz: model.motor_lpf,
        motor_lpf_from: motor_lpf_from.into(),
        bf_rough_v,
        hover_rough_v,
        delay_margin_ms,
        max_margin_ms: margin::MAX_MARGIN,
        trim_deg,
        flicks,
    })
}

/// Fly the computed gains through a full-stick flick each way per axis on the
/// plant of the fit they came from; writes and returns `gains/flicks.json`.
/// `stick_tracking` and `motor_lpf_hz` override the λ and the motor-speed
/// low-pass (0 for off) the fit's logs were flown with, and
/// `min_rise_ms` slows sticks the target limits would allow faster, and
/// `feedback` picks the feedback gains flown (all if None), in `conditions`
/// (the defaults if None).
#[tauri::command]
pub async fn whippyflight_flicks(
    workdir: String,
    max_rates: Option<MaxRates>,
    stick_tracking: Option<f32>,
    motor_lpf_hz: Option<f64>,
    min_rise_ms: Option<f64>,
    feedback: Option<FlickFeedback>,
    conditions: Option<FlickConditions>,
) -> Result<FlickReport, String> {
    let dir = self::workdir(&workdir)?;
    let min_rise = min_rise_ms.map(|ms| quantity!(ms, ms));
    tauri::async_runtime::spawn_blocking(move || {
        let report = fly_flicks(
            &dir,
            max_rates,
            stick_tracking,
            motor_lpf_hz.map(|hz| quantity!(hz, Hz)),
            min_rise,
            feedback.unwrap_or_default(),
            conditions.unwrap_or_default(),
        )?;
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
    #[serde(with = "plain")]
    dt: Time,
    #[serde(with = "plain")]
    sigma_rate: Rate,
    /// Absent from fronts computed before these were recorded.
    #[serde(default)]
    resolution: Option<u8>,
    #[serde(default, with = "plain")]
    idle_rps: Option<Rate>,
    points: Vec<FrontBlob>,
}

#[derive(Deserialize)]
struct FrontBlob {
    #[serde(with = "plain")]
    sigma_deriv: AngAccel,
    #[serde(with = "plain")]
    sigma_int: Rate,
    wfg: String,
}

/// One point of a fit's front: its weights and what they trade (`sim::margin`).
#[derive(Serialize, Deserialize, Debug)]
pub struct FrontPoint {
    #[serde(with = "plain")]
    sigma_deriv: AngAccel,
    #[serde(with = "plain")]
    sigma_int: Rate,
    /// How long a roll knock takes to die out, ∫|rate| / knock.
    #[serde(with = "plain")]
    kick_iae_ms: Millis,
    /// Motor roughness the fit's sensor noise causes at hover; None without its noise.
    #[serde(with = "plain")]
    rough_v: Option<Voltage>,
    /// As [`FlickReport::delay_margin_ms`].
    #[serde(with = "plain")]
    delay_margin_ms: Millis,
    /// As [`FlickReport::trim_deg`].
    #[serde(with = "plain")]
    trim_deg: Degrees,
}

/// `fits/<fit>/front/scores.json`.
#[derive(Serialize, Deserialize)]
pub struct FrontReport {
    version: u32,
    fit: String,
    /// The loop period and σ_rate every point's gains were computed for.
    #[serde(with = "plain")]
    dt: Time,
    #[serde(with = "plain")]
    sigma_rate: Rate,
    /// The [`Grid`] they were scheduled over; None if the front predates recording it.
    #[serde(default)]
    resolution: Option<u8>,
    #[serde(default, with = "plain")]
    idle_rps: Option<Rate>,
    /// Whether the fit's gyro filters were modeled; the measures are without them if not.
    gyro_filters: bool,
    #[serde(with = "plain")]
    max_margin_ms: Millis,
    #[serde(with = "plain")]
    bf_rough_v: Option<Voltage>,
    points: Vec<FrontPoint>,
}

fn front_index(dir: &Path, fit: &str) -> Result<(PathBuf, FrontIndex), String> {
    let front = dir.join(FITS_DIR).join(plain_name(fit)?).join(FRONT_DIR);
    let index = serde_json::from_slice(&read_file(&front.join(FRONT_INDEX))?)
        .map_err(|e| format!("{FRONT_INDEX}: {e}"))?;
    Ok((front, index))
}

fn score_front(dir: &Path, fit: &str) -> Result<FrontReport, String> {
    use whippyflight_sim::margin;

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
            kick_iae_ms: margin::kick_iae(&gains, &model, &cutoffs),
            rough_v: noise
                .as_ref()
                .map(|n| margin::hover_roughness(&gains, &model, &cutoffs, n)),
            delay_margin_ms: margin::delay_margin(&gains, &model, &cutoffs),
            trim_deg: rescale(margin::trim(&gains, &model, &cutoffs).0),
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
        max_margin_ms: margin::MAX_MARGIN,
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

    const ALL: FlickFeedback = FlickFeedback { p: true, i: true, d: true };

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
        sigma_rate: quantity!(5.0, rot / s),
        sigma_deriv: quantity!(10.0, rot / s ^ 2),
        sigma_int: quantity!(0.012, rot / s),
    };

    const GRID: Grid = Grid {
        resolution: 3,
        idle_rps: quantity!(0.0, rot / s),
    };

    #[test]
    fn parses_webview_jobs() {
        let job: Job = serde_json::from_str(
            r#"{"kind": "gen", "dt": 3.125e-4, "fit": "a", "grid": {"resolution": 4, "idleRps": 180},
                "weights": {"sigmaRate": 5, "sigmaDeriv": 15, "sigmaInt": 0.012}}"#,
        )
        .unwrap();
        let Job::Gen { dt, weights, grid, .. } = job else {
            panic!("not a gen job");
        };
        assert_eq!(dt, Some(quantity!(3.125e-4, s)));
        assert_eq!(weights.sigma_deriv, quantity!(15.0, rot / s ^ 2));
        assert_eq!(grid.idle_rps, quantity!(180.0, rot / s));
        let job: Job = serde_json::from_str(
            r#"{"kind": "front", "dt": null, "fit": "a", "sigmaRate": 5,
                "grid": {"resolution": 4, "idleRps": 0}}"#,
        )
        .unwrap();
        assert!(matches!(job, Job::Front { dt: None, .. }));
        let rates: MaxRates = serde_json::from_str(r#"{"rps": [2, 1.5, 1], "from": "fc"}"#).unwrap();
        assert_eq!(rates.rps[1], quantity!(1.5, rot / s));
        assert!(serde_json::from_str::<MaxRates>(r#"{"rps": [2, 1.5], "from": "fc"}"#).is_err());
    }

    #[test]
    fn grid_args_are_checked() {
        assert_eq!(
            grid_args(&Grid {
                resolution: 4,
                idle_rps: quantity!(130.0, rot / s)
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
                idle_rps: quantity!(-1.0, rot / s),
                ..GRID
            },
            Grid {
                idle_rps: quantity!(f64::NAN, rot / s),
                ..GRID
            },
        ] {
            assert!(grid_args(&bad).is_err(), "{bad:?}");
        }
    }

    /// Runs the real tools: WHIPPYFLIGHT_PYTHON (with the whippyflight packages) and
    /// WHIPPYFLIGHT_LOG (a .bbl).
    #[test]
    #[ignore]
    fn fits_and_generates_gains() {
        let env = |name| std::env::var(name).unwrap_or_else(|_| panic!("{name} not set"));
        let toolchain = Toolchain {
            python: env("WHIPPYFLIGHT_PYTHON"),
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
            let args = job_args(&dir, job).unwrap();
            before_job(&dir, job);
            let status = python_command(&toolchain).args(args).status().unwrap();
            assert!(status.success());
            after_job(&dir, job).unwrap();
        }
        let report = fly_flicks(&dir, None, None, None, None, ALL, FlickConditions::default()).unwrap();
        std::fs::write(dir.join(FLICKS), serde_json::to_vec(&report).unwrap()).unwrap();
        let status = python_command(&toolchain)
            .args(job_args(&dir, &Job::FlickPlots).unwrap())
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
            fit.id == "flight_1" && fit.has_coefficients && fit.has_diagnostics && fit.has_noise && fit.noise_rms.is_some()
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
        let at = |(deriv, int): (AngAccel, Rate)| {
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
        let sigmas = |deriv: f64, int: f64| (quantity!(deriv, rot / s ^ 2), quantity!(int, rot / s));
        let (stiff, smooth) = (p(at(sigmas(5.0, 0.012))), p(at(sigmas(30.0, 0.012))));
        assert!(stiff.kick_iae_ms < smooth.kick_iae_ms);
        assert!(stiff.rough_v.unwrap() > smooth.rough_v.unwrap());
        let (firm, loose) = (p(at(sigmas(15.0, 0.004))), p(at(sigmas(15.0, 0.05))));
        assert!(firm.trim_deg < loose.trim_deg);
        assert!(firm.delay_margin_ms < loose.delay_margin_ms);

        // The front's gains at the Gen job's weights are the Gen job's.
        let mine = at((WEIGHTS.sigma_deriv, WEIGHTS.sigma_int));
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
        let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../whippyflight/fixtures");
        let dir = std::env::temp_dir().join(format!("whippyflight-flicks-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("fits/meteor")).unwrap();
        std::fs::create_dir_all(dir.join("gains")).unwrap();
        let mut coefficients: serde_json::Value =
            serde_json::from_slice(&std::fs::read(tools.join("airframes/meteor65.json")).unwrap()).unwrap();
        coefficients["latency"] = serde_json::json!({"actuator_ms": 0.31, "telemetry_ms": 3.23});
        std::fs::write(dir.join("fits/meteor").join(COEFFICIENTS), coefficients.to_string()).unwrap();
        std::fs::copy(tools.join("test_gains.wfg"), dir.join(GAINS)).unwrap();
        std::fs::write(dir.join(GAINS_FIT), "meteor").unwrap();
        std::fs::copy(
            tools.join("airframes/meteor65_noise.bin"),
            dir.join("fits/meteor").join(NOISE),
        )
        .unwrap();
        std::fs::write(
            dir.join("fits/meteor").join(NOISE_INFO),
            r#"{"bf_rough_v": 0.048, "motor_noise_speed": {"reference_rps": 400.0, "exponent": 0.6}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("fits/meteor").join(DIAGNOSTICS),
            r#"{"turbulence": {"roll": {"rms": 2.0, "hz": 4.0}, "pitch": {"rms": 1.0, "hz": 2.0}, "yaw": {"rms": 0.0, "hz": 6.0}}}"#,
        )
        .unwrap();

        let started = std::time::Instant::now();
        let rates = || {
            Some(MaxRates {
                rps: [2.0, 1.5, 1.0].map(|r| quantity!(r, rot / s)),
                from: "fc".into(),
            })
        };
        let report = fly_flicks(&dir, rates(), None, None, None, ALL, FlickConditions::default()).unwrap();
        eprintln!("6 flicks in {:?}", started.elapsed());
        // The panel's stick tracking replaces the logs'.
        let retracked = fly_flicks(&dir, None, Some(1.0), None, None, ALL, FlickConditions::default()).unwrap();
        assert!(fly_flicks(&dir, None, Some(0.0), None, None, ALL, FlickConditions::default()).is_err());
        // So does its motor-speed low-pass.
        let refiltered = fly_flicks(&dir, None, None, Some(quantity!(20.0, Hz)), None, ALL, FlickConditions::default()).unwrap();
        assert!(fly_flicks(&dir, None, None, Some(quantity!(-1.0, Hz)), None, ALL, FlickConditions::default()).is_err());
        // A slower stick than the limits need slows every flick to it.
        let slowed = fly_flicks(&dir, rates(), None, None, Some(quantity!(150.0, ms)), ALL, FlickConditions::default()).unwrap();
        assert!(fly_flicks(&dir, None, None, None, Some(quantity!(0.0, ms)), ALL, FlickConditions::default()).is_err());
        // Feedforward alone: other flights, the same gains' margins.
        let none = FlickFeedback { p: false, i: false, d: false };
        let open_loop = fly_flicks(&dir, rates(), None, None, None, none, FlickConditions::default()).unwrap();
        // Proportional alone: neither the full gains' flights nor the feedforward's.
        let p_only = FlickFeedback { p: true, ..none };
        let proportional = fly_flicks(&dir, rates(), None, None, None, p_only, FlickConditions::default()).unwrap();
        let rough = FlickConditions {
            gyro_noise_rms: Some(quantity!(0.005, rot / s)),
            motor_noise: false,
            motor_speed_noise_rms: Some(quantity!(1.0, rot / s)),
            noisy_flights: true,
            cg_shift_pct: [0.0, 5.0],
            turbulence: true,
            ..FlickConditions::default()
        };
        let disturbed = fly_flicks(&dir, rates(), None, None, None, ALL, rough).unwrap();
        let set = FlickConditions {
            turbulence_rms: Some([quantity!(0.3, rot / s ^ 2); 3]),
            turbulence_hz: Some(quantity!(8.0, Hz)),
            ..rough
        };
        let set_turbulence = fly_flicks(&dir, rates(), None, None, None, ALL, set).unwrap().turbulence;
        let calm = FlickConditions { turbulence: false, ..set };
        assert_eq!(fly_flicks(&dir, rates(), None, None, None, ALL, calm).unwrap().turbulence, None);
        let bad = FlickConditions { turbulence_hz: Some(quantity!(0.0, Hz)), ..FlickConditions::default() };
        assert!(fly_flicks(&dir, rates(), None, None, None, ALL, bad).is_err());
        // The conditions' delays replace the fit's, margins included.
        let late = FlickConditions {
            actuator_delay_ms: Some(quantity!(3.0, ms)),
            telemetry_delay_ms: Some(quantity!(0.0, ms)),
            ..FlickConditions::default()
        };
        let delayed = fly_flicks(&dir, rates(), None, None, None, ALL, late).unwrap();
        let bad = FlickConditions { actuator_delay_ms: Some(quantity!(25.0, ms)), ..FlickConditions::default() };
        assert!(fly_flicks(&dir, rates(), None, None, None, ALL, bad).is_err());
        let fitted = fit_latency(&dir.join("fits/meteor")).expect("the fit's latency");
        std::fs::remove_dir_all(&dir).unwrap();

        let tick: Millis = rescale(report.loop_dt);
        let near = |a: Millis, b: Millis| (a - b).abs() <= 0.5 * tick;
        assert_eq!(fitted.telemetry_ms, quantity!(3.23, ms));
        assert!(near(report.latency.actuator_ms, fitted.actuator_ms) && near(report.latency.telemetry_ms, fitted.telemetry_ms));
        assert!(near(delayed.latency.actuator_ms, quantity!(3.0, ms)));
        assert_eq!(delayed.latency.telemetry_ms, quantity!(0.0, ms));
        assert_ne!(delayed.flicks[0].response, report.flicks[0].response);
        assert!(delayed.delay_margin_ms < report.delay_margin_ms, "the motors' delay eats the margin");

        assert_eq!((report.feedback, open_loop.feedback, proportional.feedback), (ALL, none, p_only));
        assert_ne!(open_loop.flicks[0].response, report.flicks[0].response);
        assert_ne!(proportional.flicks[0].response, report.flicks[0].response);
        assert_ne!(proportional.flicks[0].response, open_loop.flicks[0].response);
        assert_eq!(open_loop.delay_margin_ms, report.delay_margin_ms);
        assert_eq!(report.conditions, FlickConditions::default());
        assert_eq!(disturbed.conditions, rough);
        let logged = report.noise_rms.expect("flown with noise");
        assert!(value!(logged.gyro, rot / s) > 0.0 && value!(logged.motor, rot / s) > 0.0);
        assert_eq!(logged.motor_speed_exponent, Some(0.6), "noise.json's speed law");
        let asked = disturbed.noise_rms.expect("flown with noise");
        assert!((value!(asked.gyro, rot / s) - 0.005).abs() < 1e-6);
        assert_eq!(value!(asked.motor, rot / s), 0.0);
        assert_eq!(report.turbulence, None);
        let logged = disturbed.turbulence.expect("seeded from the diagnostics");
        assert!((value!(logged.rms[0], rot / s ^ 2) - 2.0).abs() < 1e-9);
        assert!((value!(logged.hz, Hz) - 3.6).abs() < 1e-9, "power-weighted corner");
        let set_turbulence = set_turbulence.expect("set");
        assert_eq!(value!(set_turbulence.hz, Hz), 8.0);
        assert!((value!(set_turbulence.rms[2], rot / s ^ 2) - 0.3).abs() < 1e-12);
        assert!(report.gyro_filtered);
        assert_ne!(disturbed.flicks[0].response, report.flicks[0].response);
        assert_eq!(disturbed.delay_margin_ms, report.delay_margin_ms);

        assert_eq!(report.stick_tracking_from, "log");
        assert_ne!(report.stick_tracking, 1.0);
        assert_eq!(retracked.stick_tracking_from, "fc");
        assert_eq!(retracked.stick_tracking, 1.0);
        assert_ne!(retracked.flicks[0].setpoint, report.flicks[0].setpoint);
        assert_eq!(report.motor_lpf_from, "log");
        assert_eq!(refiltered.motor_lpf_from, "fc");
        assert_eq!(value!(refiltered.motor_lpf_hz, Hz), 20.0);
        assert_ne!(refiltered.motor_lpf_hz, report.motor_lpf_hz);
        assert_ne!(refiltered.flicks[0].response, report.flicks[0].response);

        assert_eq!(report.fit, "meteor");
        assert!(
            report.rates_from == "fc"
                && report.slew > quantity!(0.0, rot / s ^ 2)
                && report.jerk > quantity!(0.0, rot / s ^ 3)
        );
        assert_eq!(report.bf_rough_v, Some(quantity!(0.048, V)));
        let hover = report.hover_rough_v.expect("flown with noise");
        eprintln!("hover roughness {} V", value!(hover, V));
        assert!(hover > quantity!(0.0, V) && hover < quantity!(0.048, V));
        eprintln!(
            "delay margin {:?} ms, trim {:?}°",
            report.delay_margin_ms.map(|m| value!(m, ms)),
            report.trim_deg.map(|t| value!(t, deg))
        );
        assert!(report
            .trim_deg
            .is_some_and(|t| t > quantity!(0.0, deg) && t < quantity!(45.0, deg)));
        assert!(report.delay_margin_ms.is_some_and(|m| m > quantity!(0.0, ms)));
        assert!(report.trace_dt >= 0.5 * FLICK_TRACE_DT);
        assert_eq!(
            report.flicks.iter().map(|f| value!(f.amp, rot / s)).collect::<Vec<_>>(),
            [2.0, -2.0, 1.5, -1.5, 1.0, -1.0]
        );
        assert_eq!(slowed.min_rise_ms, Some(quantity!(150.0, ms)));
        for (fast, slow) in report.flicks.iter().zip(&slowed.flicks) {
            assert_eq!(fast.rise_ms, fast.limit_rise_ms);
            assert!(fast.limit_rise_ms < quantity!(150.0, ms));
            assert_eq!(slow.rise_ms, quantity!(150.0, ms));
            assert_eq!(slow.limit_rise_ms, fast.limit_rise_ms);
        }
        // The JSON carries bare numbers in the fields' units.
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["bf_rough_v"], 0.048);
        assert_eq!(json["max_margin_ms"], value!(whippyflight_sim::margin::MAX_MARGIN, ms));
        assert_eq!(json["hold_s"], value!(whippyflight_sim::flick::HOLD, s));
        assert_eq!(json["flicks"][3]["amp"], -1.5);
        assert!(json["flicks"][0]["rise_ms"].as_f64().is_some_and(|t| t > 1.0));
        assert!(json["flicks"][0]["volts"][3][0].as_f64().is_some_and(|v| v > 0.0));
        let speeds = &json["flicks"][0]["motor_speeds"];
        assert_eq!(speeds[3].as_array().map(Vec::len), json["flicks"][0]["volts"][3].as_array().map(Vec::len));
        assert!(speeds[3][0].as_f64().is_some_and(|w| w > 0.0), "hovering, in rot/s");
        assert!(serde_json::from_value::<FlickReport>(json).is_ok());
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
            eprintln!(
                "{} {} rot/s over {:.1} ms: {m:?}",
                f.axis,
                value!(f.amp, rot / s),
                value!(f.rise_ms, ms)
            );
            // Within the limits the target stays close behind the stick.
            assert!(
                m.target_lag_ms < 0.25 * f.rise_ms && m.target_ovs.abs() < 0.03,
                "{} target: {m:?}",
                f.axis
            );
            assert!(m.rise90_ms.is_some() && m.hold_err.abs() < 0.1);
            let rough = m.rough_v.expect("flown with noise");
            assert!(
                rough > quantity!(0.0, V) && rough < quantity!(1.0, V),
                "{} roughness {} V",
                f.axis,
                value!(rough, V)
            );
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
