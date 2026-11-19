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
 * A fit's Pareto front: gains computed over a grid of σ_deriv and σ_int at the time constants
 * derived for the fit (front_wfg.py), each flown on the fit's plant for what the weights trade
 * (the controller crate's `sim::margin`).
 */

import type { TimeConstants, Weights } from "./weights";

export const FRONT_VERSION = 2;

/** One point of the front (`whippyflight.rs` FrontPoint). */
export interface FrontPoint {
    sigma_deriv: number;
    sigma_int: number;
    /** Its [σ_deriv level, σ_int level] on the grid. */
    grid: [number, number];
    /** How long a roll knock takes to die out, ∫|rate| / knock, ms. */
    kick_iae_ms: number;
    /** Motor roughness the fit's sensor noise causes at hover, V; null without its noise. */
    rough_v: number | null;
    /** Extra gyro delay the gains tolerate, ms; −1 if they don't fly cleanly even without. */
    delay_margin_ms: number;
    /** Attitude a 5% CG shift costs before the integral takes it up, deg. */
    trim_deg: number;
}

/** `fits/<fit>/front/scores.json`. */
export interface FrontReport {
    version: number;
    fit: string;
    /** The loop period, s, σ_rate and time constants, s, every point's gains were computed for. */
    dt: number;
    sigma_rate: number;
    /** σ_rate, rot/s, derived at this front's τ_int, dt and grid; absent from older fronts. */
    derived_sigma_rate?: number | null;
    tau_deriv: number;
    tau_int: number;
    /** τ_deriv, s, derived at this front's σ_rate, τ_int, dt and grid; absent from older fronts. */
    derived_tau_deriv?: number | null;
    /** The grid they were scheduled over (`Grid`); absent from fronts computed before it was recorded. */
    resolution?: number | null;
    idle_rps?: number | null;
    /** Whether the fit's gyro filters were modeled; the measures are without them if not. */
    gyro_filters: boolean;
    max_margin_ms: number;
    bf_rough_v: number | null;
    points: FrontPoint[];
}

export type FrontMeasure = "kick_iae_ms" | "rough_v" | "delay_margin_ms" | "trim_deg";

/** Whether more of a measure is better. */
export const HIGHER_IS_BETTER: Record<FrontMeasure, boolean> = {
    kick_iae_ms: false,
    rough_v: false,
    delay_margin_ms: true,
    trim_deg: false,
};

export const FRONT_MEASURES = Object.keys(HIGHER_IS_BETTER) as FrontMeasure[];

/** A measure as a cost: lower is better; null if the point lacks it. */
function cost(point: FrontPoint, measure: FrontMeasure): number | null {
    const value = point[measure];
    return value === null ? null : HIGHER_IS_BETTER[measure] ? -value : value;
}

/** Whether `a` is at least as good as `b` in each of `measures` both have, and better in one. */
export function dominates(a: FrontPoint, b: FrontPoint, measures: FrontMeasure[] = FRONT_MEASURES): boolean {
    let better = false;
    for (const measure of measures) {
        const [ca, cb] = [cost(a, measure), cost(b, measure)];
        if (ca === null || cb === null) {
            continue;
        }
        if (ca > cb) {
            return false;
        }
        better ||= ca < cb;
    }
    return better;
}

/**
 * @returns for each point, whether no other point dominates it in `measures`. Over all four, a
 * grid of two weights is mostly undominated; over a chart's two axes it's the chart's envelope.
 */
export function paretoOptimal(points: FrontPoint[], measures: FrontMeasure[] = FRONT_MEASURES): boolean[] {
    return points.map((p) => !points.some((q) => q !== p && dominates(q, p, measures)));
}

/** The point with weights `weights`, if the front has them. */
export function frontIndexOf(report: FrontReport, weights: Weights | null): number {
    if (
        !weights ||
        weights.sigmaRate !== report.sigma_rate ||
        weights.tauDeriv !== report.tau_deriv ||
        weights.tauInt !== report.tau_int
    ) {
        return -1;
    }
    return report.points.findIndex((p) => p.sigma_deriv === weights.sigmaDeriv && p.sigma_int === weights.sigmaInt);
}

/** Whether the front was computed at τ_int of `weights`, `grid` and loop period `dt`, s (null: not known yet). */
function sameSettings(
    report: FrontReport,
    weights: Weights,
    grid: { resolution: number; idleRps: number },
    dt: number | null,
): boolean {
    return (
        report.tau_int === weights.tauInt &&
        report.resolution === grid.resolution &&
        report.idle_rps === grid.idleRps &&
        (dt === null || Math.abs(report.dt - dt) <= 0.01 * dt)
    );
}

/** @returns the front's derived σ_rate, rot/s, if it was derived at these settings (as `sameSettings`); else null. */
export function derivedSigmaRate(
    report: FrontReport,
    weights: Weights,
    grid: { resolution: number; idleRps: number },
    dt: number | null,
): number | null {
    return sameSettings(report, weights, grid, dt) ? (report.derived_sigma_rate ?? null) : null;
}

/** @returns the front's derived τ_deriv, s, if it was derived at these settings and σ_rate of `weights`; else null. */
export function derivedTauDeriv(
    report: FrontReport,
    weights: Weights,
    grid: { resolution: number; idleRps: number },
    dt: number | null,
): number | null {
    return report.sigma_rate === weights.sigmaRate && sameSettings(report, weights, grid, dt)
        ? (report.derived_tau_deriv ?? null)
        : null;
}

export function frontTimeConstants(report: FrontReport): TimeConstants {
    return { tauDeriv: report.tau_deriv, tauInt: report.tau_int };
}

export function frontWeights(report: FrontReport, point: FrontPoint): Weights {
    return {
        sigmaRate: report.sigma_rate,
        sigmaDeriv: point.sigma_deriv,
        sigmaInt: point.sigma_int,
        ...frontTimeConstants(report),
    };
}
