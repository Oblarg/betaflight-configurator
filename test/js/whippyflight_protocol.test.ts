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
import { crc32, decodeStatus, dtMatches, encodeUploadData, parseGainBlob } from "../../src/js/whippyflight/protocol";
import { ArmingCheck, GainLoadResult, GainSource } from "../../src/js/msp/WhippyFlightCodes";
import { makeBlob, makeStatus } from "./whippyflight_fixtures";

describe("WhippyFlight protocol", () => {
    it("computes the zlib CRC-32 the blob trailer stores", () => {
        expect(crc32(new TextEncoder().encode("123456789"))).toBe(0xcbf43926);
        expect(crc32(new Uint8Array())).toBe(0);
    });

    it("decodes the status reply, including the arming reason", () => {
        const status = decodeStatus(
            makeStatus(
                {
                    blobSize: 16820,
                    blobCrc: 0x7c8e_4f8a,
                    uploadReceived: 256,
                    active: 0,
                    armingChecks: ArmingCheck.RPM_TELEMETRY | ArmingCheck.FAILSAFE,
                },
                "simulator sends no ESC telemetry",
            ),
        );
        expect(status).toMatchObject({
            gainResult: GainLoadResult.OK,
            source: GainSource.DATAFLASH,
            uploading: false,
            blobSize: 16820,
            blobCrc: 0x7c8e_4f8a,
            partitionSize: 20480,
            uploadReceived: 256,
            maxChunk: 128,
            active: false,
            armingChecks: ArmingCheck.RPM_TELEMETRY | ArmingCheck.FAILSAFE,
            armingBlockReason: "simulator sends no ESC telemetry",
        });
        expect(status.pidDt).toBeCloseTo(0.00025, 9);
    });

    it("refuses short or newer status replies", () => {
        expect(() => decodeStatus(new DataView(new ArrayBuffer(10)))).toThrow(/too short/);
        const newer = makeStatus();
        newer.setUint8(0, 3);
        expect(() => decodeStatus(newer)).toThrow(/version 3/);
    });

    it("encodes a data chunk as a little-endian offset then the bytes", () => {
        expect(encodeUploadData(0x01020304, new Uint8Array([9, 8]))).toEqual([4, 3, 2, 1, 9, 8]);
    });

    it("reads a valid blob's header", () => {
        const blob = makeBlob(64, 0.0003125);
        const info = parseGainBlob(blob);
        expect(info).toMatchObject({ size: blob.length, version: 7, resolution: 3, feedforwardHorizon: 8 });
        expect("dt" in info && info.dt).toBeCloseTo(0.0003125, 9);
    });

    it("names what is wrong with a bad blob", () => {
        expect(parseGainBlob(new Uint8Array(8))).toEqual({ error: "too_short" });

        const magic = makeBlob();
        magic[0] ^= 1;
        expect(parseGainBlob(magic)).toEqual({ error: "bad_magic" });

        expect(parseGainBlob(makeBlob().subarray(0, 80))).toEqual({ error: "bad_size" });

        const corrupt = makeBlob();
        corrupt[0x40] ^= 0xff;
        expect(parseGainBlob(corrupt)).toEqual({ error: "bad_checksum" });
    });

    it("matches loop periods within the firmware's 1 % arming tolerance", () => {
        expect(dtMatches(0.00025, 0.00025)).toBe(true);
        expect(dtMatches(0.000252, 0.00025)).toBe(true);
        expect(dtMatches(0.0003125, 0.00025)).toBe(false);
        expect(dtMatches(0.00025, 0)).toBe(false);
    });
});
