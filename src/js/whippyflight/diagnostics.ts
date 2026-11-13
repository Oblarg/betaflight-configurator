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

/** The fit-quality diagnostics `fit_model.py fit --diagnostics` writes (fit_diagnostics.py). */

export const DIAGNOSTICS_AXES = ["yaw", "pitch", "roll", "motors"] as const;
export type DiagnosticsAxis = (typeof DIAGNOSTICS_AXES)[number];

export interface AxisDiagnostics {
    /** Of the measured quantity: rot/s² for the rate axes, V for motors. */
    unit: string;
    rows: number;
    r2: number;
    heldout_r2: number | null;
    measured_std: number;
    residual_std: number;
    coefficients: { name: string; value: number; rel_se: number | null }[];
    scatter: { measured: number[]; predicted: number[] };
    residual_hist: { edges: number[]; counts: number[] };
    excerpt: {
        flight: string;
        /** The window's length; the fit's rows within it may not fill it. Absent from older fits. */
        window_s?: number;
        t: number[];
        measured: number[];
        predicted: number[];
    } | null;
}

export interface FlightDiagnostics {
    label: string;
    duration_s: number;
    kept_s: number;
    log_hz: number;
    gating: { ground_s?: number; impacts?: number; impact_s?: number };
    r2: Record<DiagnosticsAxis, number>;
    pairing: Record<string, Record<string, number>>;
    pairing_mismatch: string[];
}

export type DiagnosticsWarning =
    | { code: "low_r2"; axis: DiagnosticsAxis; value: number; limit: number }
    | { code: "overfit"; axis: DiagnosticsAxis; r2: number; heldout_r2: number }
    | { code: "uncertain_coefficient"; axis: DiagnosticsAxis; name: string; rel_se: number; limit: number }
    | { code: "pairing_mismatch"; flight: string; axes: string[] }
    | { code: "little_data"; kept_s: number; limit: number };

export interface Diagnostics {
    version: number;
    heldout_method: string;
    kept_s: number;
    drag_axes: string[];
    axes: Record<DiagnosticsAxis, AxisDiagnostics>;
    flights: FlightDiagnostics[];
    warnings: DiagnosticsWarning[];
}
