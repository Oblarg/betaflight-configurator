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

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const { fc } = vi.hoisted(() => ({
    fc: {
        promise: vi.fn(),
        send_message: vi.fn(),
        callbacks_cleanup: vi.fn(),
    },
}));

vi.mock("../../src/js/msp", () => ({ default: fc }));

import WhippyFlightCodes, { GainLoadResult, GainSource, UploadResult } from "../../src/js/msp/WhippyFlightCodes";
import { UploadError, useWhippyFlightStore } from "../../src/stores/whippyflight";
import { useConnectionStore } from "../../src/stores/connection";
import { crc32 } from "../../src/js/whippyflight/protocol";
import { MspCancelledError, MspTimeoutError } from "../../src/js/msp/mspErrors";
import { makeBlob, makeStatus } from "./whippyflight_fixtures";

/** A flight controller running whippyflight_glue.c's upload state machine. */
class FakeFc {
    armed = false;
    stored: Uint8Array | null = null;
    ram = new Uint8Array(0);
    uploading = false;
    received = 0;
    rejectAll = false;
    /** Answer the chunk at this offset the way a retried duplicate is answered. */
    duplicateAt: number | null = null;
    commands: number[] = [];

    status(): DataView {
        const blob = this.stored;
        return makeStatus({
            gainResult: blob ? GainLoadResult.OK : GainLoadResult.BAD_MAGIC,
            source: GainSource.DATAFLASH,
            uploading: this.uploading ? 1 : 0,
            blobSize: blob?.length ?? 0,
            blobCrc: blob ? crc32(blob.subarray(0, blob.length - 4)) : 0,
            uploadSize: this.ram.length,
            uploadReceived: this.received,
        });
    }

    handle(code: number, data: number[] | false): DataView {
        this.commands.push(code);
        const reply = (result: UploadResult, gain = GainLoadResult.OK) =>
            new DataView(new Uint8Array([result, gain]).buffer);
        const bytes = Uint8Array.from(data || []);
        const view = new DataView(bytes.buffer);
        switch (code) {
            case WhippyFlightCodes.MSP2_WHIPPYFLIGHT_STATUS:
                return this.status();
            case WhippyFlightCodes.MSP2_WHIPPYFLIGHT_UPLOAD_BEGIN:
                if (this.armed) {
                    return reply(UploadResult.ARMED);
                }
                this.ram = new Uint8Array(view.getUint32(0, true));
                this.uploading = true;
                this.received = 0;
                return reply(UploadResult.OK, GainLoadResult.NO_STORE);
            case WhippyFlightCodes.MSP2_WHIPPYFLIGHT_UPLOAD_DATA: {
                const offset = view.getUint32(0, true);
                if (offset !== this.received) {
                    return reply(UploadResult.OUT_OF_ORDER);
                }
                this.ram.set(bytes.subarray(4), offset);
                this.received += bytes.length - 4;
                if (offset === this.duplicateAt) {
                    this.duplicateAt = null;
                    return reply(UploadResult.OUT_OF_ORDER);
                }
                return reply(UploadResult.OK);
            }
            case WhippyFlightCodes.MSP2_WHIPPYFLIGHT_UPLOAD_COMMIT:
                this.uploading = false;
                if (this.rejectAll) {
                    return reply(UploadResult.REJECTED, GainLoadResult.BAD_VERSION);
                }
                this.stored = this.ram.slice();
                return reply(UploadResult.OK);
            case WhippyFlightCodes.MSP2_WHIPPYFLIGHT_UPLOAD_ABORT:
                this.uploading = false;
                return reply(UploadResult.OK);
            default:
                throw new Error(`unexpected code ${code}`);
        }
    }
}

let device: FakeFc;

beforeEach(() => {
    setActivePinia(createPinia());
    localStorage.clear();
    device = new FakeFc();
    fc.promise.mockReset().mockImplementation(async (code: number, data: number[] | false) => ({
        data: device.handle(code, data),
        unsupported: 0,
    }));
    fc.send_message
        .mockReset()
        .mockImplementation((code: number, data: number[] | false, _sent: unknown, cb: (r: unknown) => void) => {
            cb({ data: device.handle(code, data), unsupported: 0 });
            return true;
        });
    useConnectionStore().connectionValid = true;
});

describe("WhippyFlight store", () => {
    it("detects WhippyFlight firmware by its status reply", async () => {
        const store = useWhippyFlightStore();
        expect(await store.probe()).toBe(true);
        expect(store.status?.partitionSize).toBe(20480);
    });

    it("reports firmware that answers the probe as unsupported", async () => {
        fc.promise.mockResolvedValue({ data: new DataView(new ArrayBuffer(0)), unsupported: 1 });
        const store = useWhippyFlightStore();
        expect(await store.probe()).toBe(false);
        expect(store.status).toBeNull();
    });

    it("asks again when the first tab's switch clears the probe from the MSP queue", async () => {
        fc.promise.mockRejectedValueOnce(new MspCancelledError("MSP queue cleared", undefined, "cleanup"));
        const store = useWhippyFlightStore();
        expect(await store.probe()).toBe(true);
        const statusRequests = fc.promise.mock.calls.filter(
            ([code]) => code === WhippyFlightCodes.MSP2_WHIPPYFLIGHT_STATUS,
        );
        expect(statusRequests).toHaveLength(2);
    });

    it("gives up on a probe nothing answers", async () => {
        fc.promise.mockRejectedValue(new MspTimeoutError("timed out"));
        const store = useWhippyFlightStore();
        expect(await store.probe()).toBe(false);
        expect(fc.promise).toHaveBeenCalledTimes(1);
    });

    it("uploads a blob in chunks and commits it", async () => {
        const store = useWhippyFlightStore();
        const blob = makeBlob(600);
        await store.upload(blob);

        expect(device.stored).toEqual(blob);
        const chunks = fc.promise.mock.calls.filter(
            ([code]) => code === WhippyFlightCodes.MSP2_WHIPPYFLIGHT_UPLOAD_DATA,
        );
        expect(chunks).toHaveLength(Math.ceil(blob.length / 128));
        expect(store.uploadProgress).toBe(1);
        expect(store.uploading).toBe(false);
        expect(store.status?.blobSize).toBe(blob.length);
        expect(useConnectionStore().liveDataPaused).toBe(false);
    });

    it("carries on when a retried chunk is answered out of order but was taken", async () => {
        device.duplicateAt = 256;
        const store = useWhippyFlightStore();
        const blob = makeBlob(600);
        await store.upload(blob);
        expect(device.stored).toEqual(blob);
    });

    it("refuses while armed, without aborting an upload it never began", async () => {
        device.armed = true;
        const store = useWhippyFlightStore();
        const error = await store.upload(makeBlob()).catch((e: unknown) => e);
        expect(error).toBeInstanceOf(UploadError);
        expect((error as UploadError).reply?.result).toBe(UploadResult.ARMED);
        expect(device.commands).not.toContain(WhippyFlightCodes.MSP2_WHIPPYFLIGHT_UPLOAD_ABORT);
    });

    it("reports the gain result of a blob the controller rejects", async () => {
        device.rejectAll = true;
        const store = useWhippyFlightStore();
        const error = (await store.upload(makeBlob()).catch((e: unknown) => e)) as UploadError;
        expect(error.reply).toEqual({ result: UploadResult.REJECTED, gainResult: GainLoadResult.BAD_VERSION });
        expect(device.stored).toBeNull();
    });

    it("aborts the upload when a chunk is refused", async () => {
        const store = useWhippyFlightStore();
        const original = device.handle.bind(device);
        device.handle = (code, data) => {
            if (code === WhippyFlightCodes.MSP2_WHIPPYFLIGHT_UPLOAD_DATA && device.received > 0) {
                device.commands.push(code);
                return new DataView(new Uint8Array([UploadResult.NOT_STARTED, 0]).buffer);
            }
            return original(code, data);
        };
        await expect(store.upload(makeBlob(600))).rejects.toBeInstanceOf(UploadError);
        expect(device.commands).toContain(WhippyFlightCodes.MSP2_WHIPPYFLIGHT_UPLOAD_ABORT);
        expect(device.uploading).toBe(false);
    });

    it("persists its settings", async () => {
        const store = useWhippyFlightStore();
        store.settings.workdir = "/tmp/meteor65";
        await Promise.resolve();
        setActivePinia(createPinia());
        expect(useWhippyFlightStore().settings.workdir).toBe("/tmp/meteor65");
    });
});
