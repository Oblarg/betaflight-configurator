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

import { GAIN_MAGIC, crc32 } from "../../src/js/whippyflight/protocol";
import { GainLoadResult, GainSource } from "../../src/js/msp/WhippyFlightCodes";

/** A blob with gen_wfg.py's header and trailer around `tableBytes` of zeros. */
export function makeBlob(tableBytes = 64, dt = 0.00025): Uint8Array {
    const size = 0x20 + tableBytes + 4;
    const bytes = new Uint8Array(size);
    const view = new DataView(bytes.buffer);
    view.setUint32(0, GAIN_MAGIC, true);
    view.setUint16(4, 7, true);
    view.setUint8(6, 3);
    view.setUint8(7, 8);
    view.setFloat32(8, 50, true);
    view.setFloat32(0x0c, 400, true);
    view.setFloat32(0x10, dt, true);
    view.setUint32(0x1c, size, true);
    view.setUint32(size - 4, crc32(bytes.subarray(0, size - 4)), true);
    return bytes;
}

/** MSP2_WHIPPYFLIGHT_STATUS as whippyflight_msp.c writes it. */
export function makeStatus(fields: Partial<Record<string, number>> = {}, reason = ""): DataView {
    const text = new TextEncoder().encode(reason);
    const view = new DataView(new ArrayBuffer(38 + text.length));
    view.setUint8(0, 2);
    view.setUint8(1, fields.gainResult ?? GainLoadResult.OK);
    view.setUint8(2, fields.source ?? GainSource.DATAFLASH);
    view.setUint8(3, fields.uploading ?? 0);
    view.setUint32(4, fields.blobSize ?? 0, true);
    view.setUint32(8, fields.blobCrc ?? 0, true);
    view.setFloat32(12, fields.blobDt ?? 0.00025, true);
    view.setFloat32(16, fields.pidDt ?? 0.00025, true);
    view.setUint32(20, fields.partitionSize ?? 20480, true);
    view.setUint32(24, fields.uploadSize ?? 0, true);
    view.setUint32(28, fields.uploadReceived ?? 0, true);
    view.setUint16(32, fields.maxChunk ?? 128, true);
    view.setUint8(34, fields.active ?? 1);
    view.setUint16(35, fields.armingChecks ?? 0, true);
    view.setUint8(37, text.length);
    new Uint8Array(view.buffer, 38).set(text);
    return view;
}
