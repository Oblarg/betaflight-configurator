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
 * The LQR cost weights gen_wfg.py computes gains with, and the sliders over them. Each weight is
 * the excursion its state may make (Bryson's rule): smaller is a stiffer hold on that state.
 *
 * Sweeps on the Meteor65 (tools/sweep_qr.py) put the tradeoffs in two of them. σ_deriv trades
 * disturbance rejection against motor noise: stiffer kicks back faster but is rougher, and the
 * delay margin peaks near the default and falls either side. σ_int trades how firmly the integral
 * holds attitude against a CG shift against delay margin. σ_rate barely matters over 1–20 rot/s,
 * so it's left to the advanced settings.
 *
 * The derivative state's time constant changes what the weights weigh, so it's derived per fit
 * with its front (front_wfg.py) rather than traded on a slider. The integral's leak would be set
 * by how long gusts last, which the logs don't show, so it's left at a default to adjust by hand.
 */

export interface Weights {
    /** Rate excursion, rot/s. */
    sigmaRate: number;
    /** Derivative-state excursion, rot/s². */
    sigmaDeriv: number;
    /** Integral-state excursion: the attitude error the integral holds, rot. */
    sigmaInt: number;
    /** The derivative's low-pass time constant, s. */
    tauDeriv: number;
    /** The integral's leak time constant, s. */
    tauInt: number;
}

export type TimeConstants = Pick<Weights, "tauDeriv" | "tauInt">;

/** gen_wfg.py's defaults. */
export const DEFAULT_WEIGHTS: Readonly<Weights> = {
    sigmaRate: 5,
    sigmaDeriv: 15,
    sigmaInt: 0.06,
    tauDeriv: 0.02,
    tauInt: 5,
};

/** A slider's log-scale span of one weight: its value at 0 and at {@link SLIDER_MAX}. */
export type WeightRange = readonly [number, number];

/** Stiff (left) to smooth (right). */
export const STIFFNESS_RANGE: WeightRange = [5, 30];
/** Firm (left) to loose (right). */
export const INTEGRAL_RANGE: WeightRange = [0.02, 0.25];
export const SLIDER_MAX = 100;

/** Where `value` sits on a slider over `range`, clamped to its ends. */
export function sliderOf(value: number, [lo, hi]: WeightRange): number {
    const position = (SLIDER_MAX * Math.log(value / lo)) / Math.log(hi / lo);
    return Math.min(SLIDER_MAX, Math.max(0, position));
}

/** The weight at slider `position` over `range`, to 3 significant figures. */
export function weightAt(position: number, [lo, hi]: WeightRange): number {
    return Number((lo * (hi / lo) ** (position / SLIDER_MAX)).toPrecision(3));
}

export function sameTimeConstants(a: TimeConstants, b: TimeConstants): boolean {
    return a.tauDeriv === b.tauDeriv && a.tauInt === b.tauInt;
}

export function sameWeights(a: Weights, b: Weights): boolean {
    return (
        a.sigmaRate === b.sigmaRate &&
        a.sigmaDeriv === b.sigmaDeriv &&
        a.sigmaInt === b.sigmaInt &&
        sameTimeConstants(a, b)
    );
}
