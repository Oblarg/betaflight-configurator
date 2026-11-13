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

import { ArmingCheck } from "../msp/WhippyFlightCodes";
import { dtMatches, type WhippyFlightStatus } from "./protocol";

export { ArmingCheck };

/**
 * The flight controller settings the WhippyFlight panel edits, by their firmware names, and
 * whether a change waits for a reboot: the controller is chosen at boot, and BF reads the pole
 * count into its motor setup there.
 */
export const CONTROLLER_SETTINGS = {
    enabled: { name: "whippyflight_enabled", reboot: true },
    motorLpfHz: { name: "whippyflight_rpm_lpf_hz", reboot: false },
    /** In hundredths of λ ({@link STICK_TRACKING_SCALE}). */
    stickTracking: { name: "whippyflight_stick_tracking", reboot: false },
    /** Mechanical rev/s: WhippyFlight's idle and the lowest speed it schedules gains at. */
    idleRps: { name: "whippyflight_idle_rps", reboot: false },
    motorPoles: { name: "motor_poles", reboot: true },
    /** `WHIPPYFLIGHT_DEBUG_MODE` logs the controller's telemetry in the blackbox debug fields; read at boot. */
    logTelemetry: { name: "debug_mode", reboot: true },
    /** Inverted: OFF logs the accelerometer, which fits need for the airframe's drag. */
    logAccel: { name: "blackbox_disable_acc", reboot: false },
} as const;

/** The `debug_mode` that logs WhippyFlight's telemetry. */
export const WHIPPYFLIGHT_DEBUG_MODE = "WHIPPYFLIGHT";

export type ControllerSettingKey = keyof typeof CONTROLLER_SETTINGS;

/** `whippyflight_stick_tracking` per unit of λ: the firmware stores it in hundredths. */
export const STICK_TRACKING_SCALE = 100;

/** Settings an arming check's fix writes; read too, since a build may not have them. */
export const FIX_SETTINGS = ["dshot_bidir", "failsafe_procedure", "pid_process_denom"] as const;

/** A change that clears an arming check: write `value` to `setting`, then save (and reboot). */
export interface CheckFix {
    setting: string;
    value: string | number;
    reboot: boolean;
}

export interface CheckItem {
    check: ArmingCheck;
    fix: CheckFix | null;
}

/** Checks in the order the firmware reports the first failing one. */
const CHECK_ORDER = [
    ArmingCheck.GAINS,
    ArmingCheck.LOOP_RATE,
    ArmingCheck.RPM_TELEMETRY,
    ArmingCheck.SELF_LEVEL,
    ArmingCheck.FAILSAFE,
];

/**
 * The PID loop divider that runs the loop at the gains' period, from the current one.
 * @returns the divider, or null when no whole divider of the gyro rate matches
 */
export function loopDenomFor(blobDt: number, pidDt: number, denom: number): number | null {
    if (!(blobDt > 0 && pidDt > 0 && denom >= 1)) {
        return null;
    }
    const gyroDt = pidDt / denom;
    const wanted = Math.round(blobDt / gyroDt);
    return wanted >= 1 && wanted !== denom && dtMatches(blobDt, gyroDt * wanted) ? wanted : null;
}

/**
 * The failing arming checks, each with the setting change that clears it when there is one.
 * A transient upload is left out: it clears itself.
 * @param values the FC's current values of `FIX_SETTINGS`; absent or null when the build lacks one
 */
export function failingChecks(status: WhippyFlightStatus, values: Record<string, string | null>): CheckItem[] {
    return CHECK_ORDER.filter((check) => status.armingChecks & check).map((check) => {
        switch (check) {
            case ArmingCheck.LOOP_RATE: {
                const denom = loopDenomFor(status.blobDt, status.pidDt, Number(values.pid_process_denom));
                return {
                    check,
                    fix: denom === null ? null : { setting: "pid_process_denom", value: denom, reboot: true },
                };
            }
            case ArmingCheck.RPM_TELEMETRY:
                return {
                    check,
                    fix: values.dshot_bidir ? { setting: "dshot_bidir", value: "ON", reboot: true } : null,
                };
            case ArmingCheck.FAILSAFE:
                return {
                    check,
                    fix: values.failsafe_procedure
                        ? { setting: "failsafe_procedure", value: "DROP", reboot: false }
                        : null,
                };
            default:
                return { check, fix: null };
        }
    });
}
