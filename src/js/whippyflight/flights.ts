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

/** Choosing flights to fit, and naming the fit of a choice. */

import type { DragMode, FlightEntry, FlightIndex, LogFile } from "./pipeline";
import { crc32 } from "./protocol";

/** Longest fit directory name before the flight list is abbreviated. */
const MAX_FIT_ID = 96;

/** A flight in the working directory: `spec` is what fit_model.py takes, `<log>#<n>`. */
export interface Flight {
    log: string;
    spec: string;
    entry: FlightEntry;
    usable: boolean;
}

export const flightSpec = (log: string, index: number): string => `${log}#${index}`;

/** @returns the logs `index` has no current entry for: new, or changed since they were scanned */
export function unscannedLogs(index: FlightIndex | null, logs: LogFile[]): string[] {
    return logs.filter((log) => index?.[log.name]?.size !== log.size).map((log) => log.name);
}

/** @returns every scanned flight, in log then log-order */
export function listFlights(index: FlightIndex | null, logs: LogFile[]): Flight[] {
    return logs.flatMap((log) =>
        (index?.[log.name]?.size === log.size ? index[log.name].flights : []).map((entry) => ({
            log: log.name,
            spec: flightSpec(log.name, entry.index),
            entry,
            usable: !entry.reason,
        })),
    );
}

/**
 * The fit directory for a set of flights: readable for a few flights, the same for the same set
 * in any order, and distinct per drag mode so one doesn't overwrite the other.
 */
export function fitId(specs: string[], drag: DragMode): string {
    const parts = [...specs].sort().map((spec) => spec.replace(/\.(bbl|bfl)(?=#|$)/i, "").replace("#", "_"));
    let id = parts.join("+");
    if (id.length > MAX_FIT_ID) {
        const hash = crc32(new TextEncoder().encode(id)).toString(16).padStart(8, "0");
        id = `${parts[0]}+${parts.length - 1}more_${hash}`;
    }
    id = id.replaceAll(/[^A-Za-z0-9_+-]/g, "_").replace(/^_+/, "");
    return drag === "auto" ? id : `${id}_drag-${drag}`;
}

/** A diagnostics flight label (`logs/<log>#<n>`) as a flight spec. */
export function specOfLabel(label: string): string {
    return label.slice(label.lastIndexOf("/") + 1);
}
