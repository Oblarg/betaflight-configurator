/*
 * This file is part of Betaflight.
 *
 * Betaflight is free software. You can redistribute this software
 * and/or modify this software under the terms of the GNU General
 * Public License as published by the Free Software Foundation,
 * either version 3 of the License, or (at your option) any later
 * version.
 *
 * Betaflight is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
 *
 * See the GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public
 * License along with this software.
 *
 * If not, see <http://www.gnu.org/licenses/>.
 */

/**
 * The desktop side of the WhippyFlight tuning pipeline: `src-tauri/src/whippyflight.rs`.
 *
 * The regression and gain computation are Python scripts that only the Tauri shell can run, so
 * every function here needs `isTauriDesktop()`. Files live at fixed paths in one working
 * directory; see the Rust module for the layout.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Diagnostics } from "./diagnostics";
import { FLICKS_VERSION, type FlickReport, type MaxRates, type RatesSource } from "./flicks";
import type { Weights } from "./weights";
import { FRONT_VERSION, type FrontReport } from "./front";

export interface Toolchain {
    /** Python interpreter with numpy, scipy, control and frccontrol. */
    python: string;
    /** Directory holding fit_model.py and gen_wfg.py. */
    toolsDir: string;
}

export interface EnvReport {
    pythonVersion: string | null;
    missingModules: string[];
    hasMatplotlib: boolean;
    missingScripts: string[];
}

export interface LogFile {
    name: string;
    size: number;
}

/** One fit, saved under `fits/<id>`. */
export interface FitRun {
    id: string;
    hasCoefficients: boolean;
    hasDiagnostics: boolean;
    /** Whether its sensor noise has been extracted (`noise.bin`), for motor roughness. */
    hasNoise: boolean;
    /** Whether its Pareto front has been computed and scored ({@link scoreFront}). */
    hasFront: boolean;
    plots: string[];
    /** When its coefficients were written, ms since the epoch; 0 if they weren't. */
    modified: number;
}

export interface Workspace {
    logs: LogFile[];
    hasFlightIndex: boolean;
    hasCliDump: boolean;
    fits: FitRun[];
    hasGains: boolean;
    /** The fit the gains were computed from. */
    gainsFit: string | null;
    /** The weights the gains were computed with; null for gains computed before they were recorded. */
    gainsWeights: Weights | null;
    /** Whether the gains have been flown through stick flicks ({@link flyFlicks}). */
    hasFlicks: boolean;
    /** PNGs of the flicks in `gains/plots`. */
    flickPlots: string[];
}

/** One log in a blackbox file, as `fit_model.py flights` reports it. */
export interface FlightEntry {
    /** 1-based position in its file. */
    index: number;
    duration_s: number;
    /** ISO time the log started, when the FC knew the time. */
    start?: string;
    /** Airborne and impact-free; absent for logs too short to gate. */
    kept_s?: number;
    log_hz?: number;
    gating?: { ground_s?: number; impacts?: number; impact_s?: number };
    /** Seconds of each axis that clear the excitation gate: the rows a fit gets. */
    excited_s?: Record<"yaw" | "pitch" | "roll", number>;
    /** Why the fit would skip it; null or absent when usable. */
    reason?: string | null;
}

/** A blackbox header's rate profile; [roll, pitch, yaw], rates and expo in hundredths. */
export interface StickRates {
    rates_type: number;
    rc_rates: number[];
    rates: number[];
    rc_expo: number[];
    rate_limits: number[];
}

/** What a fit's `coefficients.json` says about the airframe its logs came from. */
export interface Airframe {
    /** The PID loop rate the logs were recorded at. */
    loop_hz?: number | null;
    stick_rates?: StickRates | null;
    /** The whippyflight_idle_rps the logs were recorded with, rev/s; 0 for none. */
    idle_rps?: number | null;
}

/** `flights.json`: per log file name, its size when scanned and its flights. */
export type FlightIndex = Record<string, { size: number; flights: FlightEntry[]; error?: string }>;

export type DragMode = "auto" | "on" | "off";

/** Speeds per motor of the gain grid; the firmware's gain store fits up to 4. */
export type GridResolution = 3 | 4;

export const GRID_RESOLUTIONS: readonly GridResolution[] = [3, 4];

/** The motor-speed grid gen_wfg schedules the gains over. */
export interface Grid {
    resolution: GridResolution;
    /** The FC's whippyflight_idle_rps, rev/s, where the grid starts when above gen_wfg's minimum; 0 for none. */
    idleRps: number;
}

export type Job =
    | { kind: "scan"; logs: string[] }
    /** `flights` are `<log>` (every flight in it) or `<log>#<n>`. */
    | { kind: "fit"; flights: string[]; drag: DragMode; fit: string }
    /** `dt` null: the loop rate the fit's logs were recorded at. */
    | { kind: "gen"; dt: number | null; fit: string; weights: Weights; grid: Grid }
    /** Extract the sensor noise of `flight` (a spec) into `fits/<fit>/noise.bin`. */
    | { kind: "noise"; fit: string; flight: string }
    /** Compute gains over a grid of σ_deriv and σ_int into `fits/<fit>/front`; `dt` as gen's. */
    | { kind: "front"; dt: number | null; fit: string; sigmaRate: number; grid: Grid }
    /** Plot `fits/<fit>/diagnostics.json` into `fits/<fit>/plots`. */
    | { kind: "fitPlots"; fit: string }
    /** Plot `gains/flicks.json` into `gains/plots`. */
    | { kind: "flickPlots" };

export interface OutputLine {
    line: string;
    stderr: boolean;
}

export const CLI_DUMP_FILE = "cli_dump.txt";
export const FLIGHT_INDEX_FILE = "flights.json";
export const GAINS_FILE = "gains/gains.wfg";
export const FLICKS_FILE = "gains/flicks.json";

export const fitFile = (fit: string, file: "coefficients.json" | "diagnostics.json"): string => `fits/${fit}/${file}`;

/** @returns what the toolchain is missing */
export function checkEnv(toolchain: Toolchain): Promise<EnvReport> {
    return invoke<EnvReport>("whippyflight_check_env", { toolchain });
}

/** @returns the working directory's contents */
export function readWorkspace(workdir: string): Promise<Workspace> {
    return invoke<Workspace>("whippyflight_workspace", { workdir });
}

/**
 * Run one job, streaming its output to `onOutput`.
 * @returns the script's exit code; rejects with "cancelled" when {@link cancelJob} killed it
 */
export async function runJob(
    toolchain: Toolchain,
    workdir: string,
    job: Job,
    onOutput: (line: OutputLine) => void,
): Promise<number> {
    const unlisten: UnlistenFn = await listen<OutputLine>("whippyflight-output", (event) => onOutput(event.payload));
    try {
        return await invoke<number>("whippyflight_run", { toolchain, workdir, job });
    } finally {
        unlisten();
    }
}

export function cancelJob(): Promise<void> {
    return invoke("whippyflight_cancel");
}

/**
 * @returns a pipeline file's bytes: one of the `*_FILE` paths, `logs/<name>`, a {@link fitFile}
 * or `fits/<fit>/plots/<name>`
 */
export async function readFile(workdir: string, file: string): Promise<Uint8Array> {
    const data = await invoke<ArrayBuffer | number[]>("whippyflight_read", { workdir, file });
    return data instanceof ArrayBuffer ? new Uint8Array(data) : Uint8Array.from(data);
}

async function readJson<T>(workdir: string, file: string): Promise<T> {
    return JSON.parse(new TextDecoder().decode(await readFile(workdir, file))) as T;
}

export function readDiagnostics(workdir: string, fit: string): Promise<Diagnostics> {
    return readJson<Diagnostics>(workdir, fitFile(fit, "diagnostics.json"));
}

/** @returns the `airframe` block of a fit's coefficients */
export async function readAirframe(workdir: string, fit: string): Promise<Airframe> {
    const coefficients = await readJson<{ airframe?: Airframe }>(workdir, fitFile(fit, "coefficients.json"));
    return coefficients.airframe ?? {};
}

export function readFlightIndex(workdir: string): Promise<FlightIndex> {
    return readJson<FlightIndex>(workdir, FLIGHT_INDEX_FILE);
}

/** @returns null for a report of an older layout */
export async function readFront(workdir: string, fit: string): Promise<FrontReport | null> {
    const report = await readJson<FrontReport>(workdir, `fits/${fit}/front/scores.json`);
    return report.version === FRONT_VERSION ? report : null;
}

/** Fly every gain set of a fit's front (the "front" job's) for what it trades; writes its scores. */
export function scoreFront(workdir: string, fit: string): Promise<FrontReport> {
    return invoke<FrontReport>("whippyflight_front", { workdir, fit });
}

/** Make point `index` of a fit's front the gains, as if computed with its weights. */
export function applyFrontPoint(workdir: string, fit: string, index: number): Promise<void> {
    return invoke("whippyflight_use_front", { workdir, fit, index });
}

/** @returns null for a report of an older layout */
export async function readFlicks(workdir: string): Promise<FlickReport | null> {
    const report = await readJson<FlickReport>(workdir, FLICKS_FILE);
    return report.version === FLICKS_VERSION ? report : null;
}

/**
 * Fly the computed gains through a full-stick flick per axis on the plant of the fit they came
 * from, to `maxRates` (Betaflight's defaults if null), with stick tracking `stickTracking` (the
 * fit's logged value if null); writes {@link FLICKS_FILE}.
 */
export function flyFlicks(
    workdir: string,
    maxRates: { rps: MaxRates; from: RatesSource } | null,
    stickTracking: number | null = null,
): Promise<FlickReport> {
    return invoke<FlickReport>("whippyflight_flicks", { workdir, maxRates, stickTracking });
}

/**
 * Copy picked files into the working directory: blackbox logs into `logs/`, or a CLI dump as
 * {@link CLI_DUMP_FILE}.
 * @returns the names the logs got there (a clashing name gets a `_2` suffix)
 */
export function importFiles(workdir: string, paths: string[], dump: boolean): Promise<string[]> {
    return invoke<string[]>("whippyflight_import", { workdir, paths, dump });
}

/** @returns the blackbox logs, or the one CLI dump, the user picked; empty when cancelled */
export async function pickFiles(dump: boolean): Promise<string[]> {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const filters = dump
        ? [{ name: "CLI dump", extensions: ["txt"] }]
        : [{ name: "Blackbox logs", extensions: ["bbl", "bfl", "txt", "BBL", "BFL", "TXT"] }];
    const picked = await open({ multiple: !dump, directory: false, filters });
    if (picked === null) {
        return [];
    }
    return Array.isArray(picked) ? picked : [picked];
}

/** Write a log (`logs/<name>`) or {@link CLI_DUMP_FILE}. */
export function writeFile(workdir: string, file: string, contents: Uint8Array): Promise<void> {
    return invoke("whippyflight_write", contents, {
        headers: { workdir: encodeURIComponent(workdir), file: encodeURIComponent(file) },
    });
}

export function deleteLog(workdir: string, name: string): Promise<void> {
    return invoke("whippyflight_delete_log", { workdir, name });
}

export function deleteFit(workdir: string, fit: string): Promise<void> {
    return invoke("whippyflight_delete_fit", { workdir, fit });
}

/**
 * Open the working directory, or `logs`, `fits/<fit>`, `fits/<fit>/plots` or `gains/plots` in it,
 * in the file manager.
 */
export function reveal(workdir: string, sub?: string): Promise<void> {
    return invoke("whippyflight_reveal", { workdir, sub: sub ?? null });
}

/** @returns the directory the user picked, or null when cancelled */
export async function pickWorkdir(current?: string): Promise<string | null> {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ directory: true, multiple: false, ...(current ? { defaultPath: current } : {}) });
    return typeof picked === "string" ? picked : null;
}
