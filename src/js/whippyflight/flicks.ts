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
 * Computed gains flown through a full-stick flick per axis on the fit's plant, by the
 * controller crate itself (`whippyflight_control::sim::flick`): `gains/flicks.json`, from
 * `whippyflight_flicks`. The stick is the fastest raised cosine within the setpoint target's slew
 * and jerk limits, so the target stays close behind it and the response shows the controller.
 */

export const FLICK_AXES = ["roll", "pitch", "yaw"] as const;
export type FlickAxis = (typeof FLICK_AXES)[number];

/** Ratios are of the flick's amplitude; times in ms. */
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
}

/** The {@link FlickReport} layout this reads; older files are flown again. */
export const FLICKS_VERSION = 7;

export interface FlickReport {
    version: number;
    /** The fit whose plant the gains were flown on. */
    fit: string;
    loop_dt: number;
    trace_dt: number;
    /** How long the stick holds before releasing, s after it gets there. */
    hold_s: number;
    /** The target limits, rot/s² and rot/s³. */
    slew: number;
    jerk: number;
    /** Where the amplitudes, the full-stick rates, came from. */
    rates_from: RatesSource | "default";
    /** The stick estimator's tracking index (λ) flown. */
    stick_tracking: number;
    /** Where it came from: the settings panel ("fc") or the fit's blackbox header ("log"). */
    stick_tracking_from: RatesSource;
    /**
     * BF's own motor roughness, V, over the stretch of the log the noise came from: all the
     * ripple in its motor outputs at hover, its response to real turbulence as well as to noise.
     */
    bf_rough_v: number | null;
    /** What the noise adds to these gains' motor voltages at hover, V: the figure to set beside `bf_rough_v`. */
    hover_rough_v: number | null;
    /**
     * Extra gyro delay, ms, the gains tolerate on top of the fit's gyro filters; −1 if they don't
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
