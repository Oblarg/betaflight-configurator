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

import RateCurve, { axisRateCurveParams } from "@/js/RateCurve";
import type { StickRates } from "./pipeline";

/**
 * Computed gains flown through a full-stick flick each way per axis on the fit's plant, by the
 * controller crate itself (`whippyflight_sim::flick`): `gains/flicks.json`, from
 * `whippyflight_flicks`. The stick is the fastest raised cosine within the setpoint target's slew
 * and jerk limits, so the target stays close behind it and the response shows the controller, or
 * a slower one if asked for.
 */

export const FLICK_AXES = ["roll", "pitch", "yaw"] as const;
export type FlickAxis = (typeof FLICK_AXES)[number];

/** In the flick's own direction: ratios are of the amplitude's magnitude; times in ms. */
export interface FlickMetrics {
    target_ovs: number;
    target_lag_ms: number;
    gyro_lag_ms: number;
    /** null when the response never reached 90% before the stick released. */
    rise90_ms: number | null;
    overshoot: number;
    /** Mean error over the last 100 ms of the hold. */
    hold_err: number;
    /** RMS response − setpoint through the release and 50 ms after. */
    track_rms: number;
    residual: number;
    /** Fraction of the flick the motors saturated. */
    sat_frac: number;
    /**
     * What the fit's logged sensor noise adds to the motor voltages over the flick: RMS of the
     * 50 Hz-high-passed difference from the noise-free flight, mean over motors, V. null when the
     * fit has no noise extracted.
     */
    rough_v: number | null;
}

/** Traces in rot/s, every `trace_dt` s from the stick's start. */
export interface FlickResult {
    axis: FlickAxis;
    rise_ms: number;
    /** The fastest rise the target limits allow; `rise_ms` is longer when a slower stick was asked for. */
    limit_rise_ms: number;
    /** FRD, rot/s: negative for the flick the other way (roll left, pitch down, yaw left). */
    amp: number;
    metrics: FlickMetrics;
    stick: number[];
    setpoint: number[];
    target: number[];
    /** Gyro with the stick minus without it. */
    response: number[];
    /** Per sample, the fraction of controller ticks the motors saturated. */
    saturated: number[];
    /** Each motor's commanded voltage (noise-free), V. */
    volts: [number[], number[], number[], number[]];
    /** Each motor's shaft speed, rot/s (the plant's, without telemetry noise). */
    motor_speeds: [number[], number[], number[], number[]];
}

/** The {@link FlickReport} layout this reads; older files are flown again. */
export const FLICKS_VERSION = 18;

/** The longest motor-command or speed-telemetry delay the flicks can be asked to fly, ms. */
export const MAX_FLICK_DELAY_MS = 20;

/** What the flicks fly in besides the stick. */
export interface FlickConditions {
    /** The fit's gyro low-pass filters between the body rates and the controller. */
    gyro_filter: boolean;
    /** The fit's recorded gyro and motor-speed noise, each on or off; their RMS is kept either way. */
    gyro_noise: boolean;
    motor_noise: boolean;
    /**
     * RMS of the gyro noise over its axes and of the motor-speed noise over the motors, rot/s:
     * the fit's recording rescaled; null for as logged.
     */
    gyro_noise_rms: number | null;
    motor_speed_noise_rms: number | null;
    /** Fly the plotted flights with the noise; otherwise it only sets the roughness. */
    noisy_flights: boolean;
    /** CG offset [right, forward], % of the arm. */
    cg_shift_pct: [number, number];
    turbulence: boolean;
    /** Turbulence RMS angular acceleration [roll, pitch, yaw], rot/s², and its bandwidth, Hz; null for the fit's. */
    turbulence_rms: [number, number, number] | null;
    turbulence_hz: number | null;
    /** Motor-command and speed-telemetry delays, ms; null for the fit's. They set the plant, margins included. */
    actuator_delay_ms: number | null;
    telemetry_delay_ms: number | null;
}

/** Dead time between a motor command and the motors acting on it, and between their speed and its report, ms. */
export interface Latency {
    actuator_ms: number;
    telemetry_ms: number;
}

/** RMS of a sensor noise recording: the gyro over its axes, the motor speed over the motors, rot/s. */
export interface NoiseRms {
    gyro: number;
    /** At the recording's motor speed. */
    motor: number;
    /** How the motor-speed noise grows with each motor's speed: as speed^this; null for alike at every speed. */
    motor_speed_exponent: number | null;
}

/** Turbulence: RMS angular acceleration [roll, pitch, yaw], rot/s², and one bandwidth, Hz. */
export interface TurbulenceSpec {
    rms: [number, number, number];
    hz: number;
}

export const DEFAULT_FLICK_CONDITIONS: Readonly<FlickConditions> = Object.freeze({
    gyro_filter: true,
    gyro_noise: true,
    motor_noise: true,
    gyro_noise_rms: null,
    motor_speed_noise_rms: null,
    noisy_flights: false,
    cg_shift_pct: [0, 0] as [number, number],
    turbulence: false,
    turbulence_rms: null,
    turbulence_hz: null,
    actuator_delay_ms: null,
    telemetry_delay_ms: null,
});

function sameValues(a: readonly number[] | null, b: readonly number[] | null): boolean {
    return a === b || (a !== null && b !== null && a.length === b.length && a.every((x, i) => x === b[i]));
}

export function sameConditions(a: FlickConditions, b: FlickConditions): boolean {
    return (
        a.gyro_filter === b.gyro_filter &&
        a.gyro_noise === b.gyro_noise &&
        a.motor_noise === b.motor_noise &&
        a.gyro_noise_rms === b.gyro_noise_rms &&
        a.motor_speed_noise_rms === b.motor_speed_noise_rms &&
        a.noisy_flights === b.noisy_flights &&
        sameValues(a.cg_shift_pct, b.cg_shift_pct) &&
        a.turbulence === b.turbulence &&
        sameValues(a.turbulence_rms, b.turbulence_rms) &&
        a.turbulence_hz === b.turbulence_hz &&
        a.actuator_delay_ms === b.actuator_delay_ms &&
        a.telemetry_delay_ms === b.telemetry_delay_ms
    );
}

/** `stored` as conditions the flicks command accepts, defaults filling in what isn't. */
export function validFlickConditions(stored: unknown): FlickConditions {
    const s = (stored ?? {}) as Partial<Record<keyof FlickConditions, unknown>>;
    const d = DEFAULT_FLICK_CONDITIONS;
    const num = <F>(v: unknown, lo: number, hi: number, fallback: F) =>
        typeof v === "number" && Number.isFinite(v) && v >= lo && v <= hi ? v : fallback;
    const bool = (v: unknown, fallback: boolean) => (typeof v === "boolean" ? v : fallback);
    const cg = Array.isArray(s.cg_shift_pct) ? s.cg_shift_pct : [];
    const rms = Array.isArray(s.turbulence_rms) && s.turbulence_rms.length === 3 ? s.turbulence_rms : [];
    const turbulenceRms = rms.map((x) => num(x, 0, Number.MAX_VALUE, null));
    return {
        gyro_filter: bool(s.gyro_filter, d.gyro_filter),
        gyro_noise: bool(s.gyro_noise, d.gyro_noise),
        motor_noise: bool(s.motor_noise, d.motor_noise),
        gyro_noise_rms: num(s.gyro_noise_rms, 0, Number.MAX_VALUE, null),
        motor_speed_noise_rms: num(s.motor_speed_noise_rms, 0, Number.MAX_VALUE, null),
        noisy_flights: bool(s.noisy_flights, d.noisy_flights),
        cg_shift_pct: [num(cg[0], -50, 50, 0), num(cg[1], -50, 50, 0)],
        turbulence: bool(s.turbulence, d.turbulence),
        turbulence_rms:
            turbulenceRms.length === 3 && turbulenceRms.every((x) => x !== null)
                ? (turbulenceRms as [number, number, number])
                : null,
        turbulence_hz: num(s.turbulence_hz, 0.1, 1000, null),
        actuator_delay_ms: num(s.actuator_delay_ms, 0, MAX_FLICK_DELAY_MS, null),
        telemetry_delay_ms: num(s.telemetry_delay_ms, 0, MAX_FLICK_DELAY_MS, null),
    };
}

/** Which feedback gains the flicks fly with; all off for the feedforward alone. */
export interface FlickFeedback {
    p: boolean;
    i: boolean;
    d: boolean;
}

export const FEEDBACK_TERMS = ["p", "i", "d"] as const;

export const DEFAULT_FLICK_FEEDBACK: Readonly<FlickFeedback> = Object.freeze({ p: true, i: true, d: true });

export function sameFeedback(a: FlickFeedback, b: FlickFeedback): boolean {
    return FEEDBACK_TERMS.every((t) => a[t] === b[t]);
}

/**
 * `stored` as feedback the flicks command accepts, all on filling in what isn't; `feedforwardOnly`,
 * the setting it replaced, turns them all off.
 */
export function validFlickFeedback(stored: unknown, feedforwardOnly?: unknown): FlickFeedback {
    if (feedforwardOnly === true && stored === undefined) {
        return { p: false, i: false, d: false };
    }
    const s = (stored ?? {}) as Partial<Record<keyof FlickFeedback, unknown>>;
    const [p, i, d] = FEEDBACK_TERMS.map((t) => (typeof s[t] === "boolean" ? (s[t] as boolean) : true));
    return { p, i, d };
}

export interface FlickReport {
    version: number;
    /** The fit whose plant the gains were flown on. */
    fit: string;
    /** The slowest stick asked for, ms; null for the fastest the limits allow. */
    min_rise_ms: number | null;
    /** The feedback gains flown; those off were zeroed. The margins, trim and hover roughness are the full gains'. */
    feedback: FlickFeedback;
    /** What they were asked to fly in. */
    conditions: FlickConditions;
    /** The delays flown (the fit's, or the conditions'), in whole controller ticks. */
    latency: Latency;
    /** The sensor noise they flew in; null without the fit's noise. */
    noise_rms: NoiseRms | null;
    /** The turbulence they flew in; null for none. */
    turbulence: TurbulenceSpec | null;
    /** Whether the gyro went through the fit's filters: asked, and modeled. */
    gyro_filtered: boolean;
    loop_dt: number;
    trace_dt: number;
    /** How long the stick holds before releasing, s after it gets there. */
    hold_s: number;
    /** The target limits, rot/s² and rot/s³. */
    slew: number;
    jerk: number;
    /** Where the amplitudes, the full-stick rates, came from. */
    rates_from: FlickRatesSource | "default";
    /** The stick estimator's tracking index (λ) flown. */
    stick_tracking: number;
    /** Where it came from: the settings panel ("fc") or the fit's blackbox header ("log"). */
    stick_tracking_from: RatesSource;
    /** The motor-speed low-pass flown, Hz (0 for off), and where it came from, as `stick_tracking_from`. */
    motor_lpf_hz: number;
    motor_lpf_from: RatesSource;
    /**
     * BF's own motor roughness, V, over the stretch of the log the noise came from: all the
     * ripple in its motor outputs at hover, its response to real turbulence as well as to noise.
     */
    bf_rough_v: number | null;
    /** What the noise adds to these gains' motor voltages at hover, V: the figure to set beside `bf_rough_v`. */
    hover_rough_v: number | null;
    /**
     * Extra gyro delay, ms, the gains tolerate on top of the fit's gyro filters and `latency`; −1 if they don't
     * fly cleanly even without, null if the filters aren't modeled. At most `max_margin_ms`.
     */
    delay_margin_ms: number | null;
    max_margin_ms: number;
    /** Attitude, deg, a 5% CG shift costs before the integral takes up the torque. */
    trim_deg: number | null;
    flicks: FlickResult[];
}

/** Roll, pitch and yaw full-stick rates, rot/s. */
export type MaxRates = [number, number, number];

/** The connected flight controller's rate profile, or a blackbox log header's. */
export type RatesSource = "fc" | "log";

/** Where the flicks' rates come from: a rate profile, or set by hand in the tab. */
export type FlickRatesSource = RatesSource | "manual";

/** Roll, pitch and yaw flick rates set by hand, rot/s. */
export type ManualFlickRates = [number, number, number];

/** Whether `report` was flown at `manual` (null: at a rate profile's rates). */
export function flownAtRates(report: FlickReport, manual: ManualFlickRates | null): boolean {
    if (manual === null) {
        return report.rates_from !== "manual";
    }
    return (
        report.rates_from === "manual" &&
        report.flicks.every((f) => Math.abs(Math.abs(f.amp) - manual[FLICK_AXES.indexOf(f.axis)]) < 1e-6)
    );
}

/** Each axis's rate in `report`, rot/s. */
export function flownRates(report: FlickReport): ManualFlickRates {
    return FLICK_AXES.map((axis) => {
        const f = report.flicks.find((flick) => flick.axis === axis);
        return f ? Math.abs(f.amp) : Number.NaN;
    }) as ManualFlickRates;
}

/** The longest of the flicks' fastest allowed rises, ms: the slowest stick the limits need. */
export function limitRiseMs(report: FlickReport): number {
    return Math.max(...report.flicks.map((f) => f.limit_rise_ms));
}

/** Full-stick rate of each axis of `curve`'s rate profile. */
export function maxRatesRps(curve: RateCurve): MaxRates {
    const rates = curve.getCurrentRates();
    const max = (axis: FlickAxis) => (curve.getMaxAngularVel(axisRateCurveParams(rates, axis)) ?? Number.NaN) / 360;
    return [max("roll"), max("pitch"), max("yaw")];
}

/** Full-stick rates of a blackbox header's rate profile, which logs MSP's values ×100. */
export function logMaxRatesRps(rates: StickRates): MaxRates {
    const hundredths = (values: number[], axis: number) => values[axis] / 100;
    const curve = new RateCurve(false, {
        rates_type: rates.rates_type,
        RC_RATE: hundredths(rates.rc_rates, 0),
        rcPitchRate: hundredths(rates.rc_rates, 1),
        rcYawRate: hundredths(rates.rc_rates, 2),
        roll_rate: hundredths(rates.rates, 0),
        pitch_rate: hundredths(rates.rates, 1),
        yaw_rate: hundredths(rates.rates, 2),
        RC_EXPO: hundredths(rates.rc_expo, 0),
        RC_PITCH_EXPO: hundredths(rates.rc_expo, 1),
        RC_YAW_EXPO: hundredths(rates.rc_expo, 2),
        roll_rate_limit: rates.rate_limits[0],
        pitch_rate_limit: rates.rate_limits[1],
        yaw_rate_limit: rates.rate_limits[2],
        deadband: 0,
        yaw_deadband: 0,
    });
    return maxRatesRps(curve);
}

/** How long the gyro trails the feedforward target at half the flick: the controller's own lag. */
export function controllerLagMs(metrics: FlickMetrics): number {
    return metrics.gyro_lag_ms - metrics.target_lag_ms;
}
