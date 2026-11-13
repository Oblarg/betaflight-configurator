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

import type { GainLoadResult, GainSource, UploadResult } from "../msp/WhippyFlightCodes";

/** The status protocol version this decoder reads. */
export const STATUS_VERSION = 2;

/** MSP2_WHIPPYFLIGHT_STATUS, decoded. Sizes in bytes, periods in seconds. */
export interface WhippyFlightStatus {
    version: number;
    gainResult: GainLoadResult;
    source: GainSource;
    uploading: boolean;
    blobSize: number;
    blobCrc: number;
    blobDt: number;
    pidDt: number;
    partitionSize: number;
    uploadSize: number;
    uploadReceived: number;
    maxChunk: number;
    /** Whether this boot flies WhippyFlight rather than Betaflight's PID controller. */
    active: boolean;
    /** The arming checks that fail, as `ArmingCheck` bits; 0 when WhippyFlight would arm. */
    armingChecks: number;
    /** Why arming is refused (the first failing check), or "" when WhippyFlight would allow it. */
    armingBlockReason: string;
}

/** The reply to every upload command. */
export interface UploadReply {
    result: UploadResult;
    gainResult: GainLoadResult;
}

const STATUS_FIXED_BYTES = 38;

/**
 * @param view the reply payload
 * @returns the decoded status
 * @throws {Error} when the payload is short or from a newer protocol
 */
export function decodeStatus(view: DataView): WhippyFlightStatus {
    if (view.byteLength < STATUS_FIXED_BYTES) {
        throw new Error(`WhippyFlight status too short: ${view.byteLength} bytes`);
    }
    const version = view.getUint8(0);
    if (version !== STATUS_VERSION) {
        throw new Error(`Unsupported WhippyFlight status version ${version}`);
    }
    const reasonLength = view.getUint8(37);
    const reasonBytes = new Uint8Array(
        view.buffer,
        view.byteOffset + STATUS_FIXED_BYTES,
        Math.min(reasonLength, view.byteLength - STATUS_FIXED_BYTES),
    );
    return {
        version,
        gainResult: view.getUint8(1),
        source: view.getUint8(2),
        uploading: view.getUint8(3) !== 0,
        blobSize: view.getUint32(4, true),
        blobCrc: view.getUint32(8, true),
        blobDt: view.getFloat32(12, true),
        pidDt: view.getFloat32(16, true),
        partitionSize: view.getUint32(20, true),
        uploadSize: view.getUint32(24, true),
        uploadReceived: view.getUint32(28, true),
        maxChunk: view.getUint16(32, true),
        active: view.getUint8(34) !== 0,
        armingChecks: view.getUint16(35, true),
        armingBlockReason: new TextDecoder().decode(reasonBytes),
    };
}

/**
 * @param view the reply payload
 * @returns the decoded reply
 */
export function decodeUploadReply(view: DataView): UploadReply {
    if (view.byteLength < 2) {
        throw new Error(`WhippyFlight upload reply too short: ${view.byteLength} bytes`);
    }
    return { result: view.getUint8(0), gainResult: view.getUint8(1) };
}

/** @returns UPLOAD_BEGIN's payload */
export function encodeUploadBegin(size: number): number[] {
    return u32(size);
}

/** @returns UPLOAD_DATA's payload: the chunk's offset, then the chunk */
export function encodeUploadData(offset: number, chunk: Uint8Array): number[] {
    return [...u32(offset), ...chunk];
}

function u32(value: number): number[] {
    return [value & 0xff, (value >>> 8) & 0xff, (value >>> 16) & 0xff, (value >>> 24) & 0xff];
}

const CRC_TABLE = (() => {
    const table = new Uint32Array(256);
    for (let n = 0; n < 256; n++) {
        let c = n;
        for (let k = 0; k < 8; k++) {
            c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
        }
        table[n] = c >>> 0;
    }
    return table;
})();

/** @returns the zlib/IEEE CRC-32 of `bytes`, as the blob trailer stores it */
export function crc32(bytes: Uint8Array): number {
    let crc = 0xffffffff;
    for (const byte of bytes) {
        crc = CRC_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
    }
    return (crc ^ 0xffffffff) >>> 0;
}

export const GAIN_MAGIC = 0x4c515247;
const GAIN_HEADER_BYTES = 0x20;

/** The header of a `.wfg` gain blob (gen_wfg.py), for display and pre-upload checks. */
export interface GainBlobInfo {
    size: number;
    crc: number;
    version: number;
    resolution: number;
    feedforwardHorizon: number;
    minRps: number;
    maxRps: number;
    dt: number;
}

export type GainBlobError = "too_short" | "bad_magic" | "bad_size" | "bad_checksum";

/**
 * Check a gain blob the way the firmware's loader starts to: magic, declared size and the
 * trailing CRC. Version and table layout are left to the firmware, which reports them.
 *
 * @returns the header, or why the bytes are not a gain blob
 */
export function parseGainBlob(bytes: Uint8Array): GainBlobInfo | { error: GainBlobError } {
    if (bytes.length < GAIN_HEADER_BYTES + 4) {
        return { error: "too_short" };
    }
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    if (view.getUint32(0, true) !== GAIN_MAGIC) {
        return { error: "bad_magic" };
    }
    if (view.getUint32(0x1c, true) !== bytes.length) {
        return { error: "bad_size" };
    }
    const crc = view.getUint32(bytes.length - 4, true);
    if (crc32(bytes.subarray(0, bytes.length - 4)) !== crc) {
        return { error: "bad_checksum" };
    }
    return {
        size: bytes.length,
        crc,
        version: view.getUint16(4, true),
        resolution: view.getUint8(6),
        feedforwardHorizon: view.getUint8(7),
        minRps: view.getFloat32(8, true),
        maxRps: view.getFloat32(0x0c, true),
        dt: view.getFloat32(0x10, true),
    };
}

/**
 * Whether a blob's loop period suits the FC's PID loop; the firmware refuses to arm beyond 1 %.
 * @returns true when within tolerance
 */
export function dtMatches(blobDt: number, pidDt: number): boolean {
    return pidDt > 0 && Math.abs(blobDt - pidDt) <= 0.01 * pidDt;
}
