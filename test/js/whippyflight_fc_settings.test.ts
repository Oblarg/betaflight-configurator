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

import { describe, expect, it } from "vitest";
import { ArmingCheck } from "../../src/js/msp/WhippyFlightCodes";
import { failingChecks, loopDenomFor } from "../../src/js/whippyflight/fcSettings";
import { decodeStatus } from "../../src/js/whippyflight/protocol";
import { makeStatus } from "./whippyflight_fixtures";

const ALL = ArmingCheck.GAINS | ArmingCheck.RPM_TELEMETRY | ArmingCheck.SELF_LEVEL | ArmingCheck.FAILSAFE;
const BOARD = { dshot_bidir: "OFF", failsafe_procedure: "AUTO-LAND", pid_process_denom: "1" };

describe("WhippyFlight arming checklist", () => {
    it("lists every failing check in the firmware's order, with the fixes this build has", () => {
        const status = decodeStatus(makeStatus({ armingChecks: ALL | ArmingCheck.UPLOADING }));
        expect(failingChecks(status, BOARD)).toEqual([
            { check: ArmingCheck.GAINS, fix: null },
            { check: ArmingCheck.RPM_TELEMETRY, fix: { setting: "dshot_bidir", value: "ON", reboot: true } },
            { check: ArmingCheck.SELF_LEVEL, fix: null },
            { check: ArmingCheck.FAILSAFE, fix: { setting: "failsafe_procedure", value: "DROP", reboot: false } },
        ]);
    });

    it("offers no fix for a setting the build lacks", () => {
        // SITL has no DShot: its missing telemetry is the simulator's.
        const status = decodeStatus(makeStatus({ armingChecks: ArmingCheck.RPM_TELEMETRY }));
        expect(failingChecks(status, { dshot_bidir: null })).toEqual([{ check: ArmingCheck.RPM_TELEMETRY, fix: null }]);
    });

    it("sets the PID loop divider that runs the loop at the gains' rate", () => {
        // 8 kHz gyro: gains for 4 kHz on a loop at 8 kHz, and for 8 kHz on one at 2 kHz.
        expect(loopDenomFor(1 / 4000, 1 / 8000, 1)).toBe(2);
        expect(loopDenomFor(1 / 8000, 1 / 2000, 4)).toBe(1);
        const status = decodeStatus(
            makeStatus({ armingChecks: ArmingCheck.LOOP_RATE, blobDt: 1 / 4000, pidDt: 1 / 8000 }),
        );
        expect(failingChecks(status, BOARD)[0].fix).toEqual({ setting: "pid_process_denom", value: 2, reboot: true });
    });

    it("offers no divider when none of the gyro rate matches the gains", () => {
        // 3.2 kHz gains on an 8 kHz gyro; 8 kHz gains on a 3.2 kHz loop would need a faster gyro.
        expect(loopDenomFor(1 / 3200, 1 / 8000, 1)).toBeNull();
        expect(loopDenomFor(1 / 8000, 1 / 3200, 1)).toBeNull();
        expect(loopDenomFor(1 / 4000, 0, 1)).toBeNull();
    });
});
