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

import { defineStore } from "pinia";
import { computed, reactive, ref, shallowRef, watch } from "vue";
import MSP from "@/js/msp";
import { MspCancelledError } from "@/js/msp/mspErrors";
import WhippyFlightCodes, { GainLoadResult, GainSource, UploadResult } from "@/js/msp/WhippyFlightCodes";
import {
    decodeStatus,
    decodeUploadReply,
    encodeUploadBegin,
    encodeUploadData,
    type UploadReply,
    type WhippyFlightStatus,
} from "@/js/whippyflight/protocol";
import {
    GRID_RESOLUTIONS,
    type Airframe,
    type DragMode,
    type EnvReport,
    type FlightIndex,
    type GridResolution,
    type OutputLine,
    type Workspace,
} from "@/js/whippyflight/pipeline";
import type { Diagnostics } from "@/js/whippyflight/diagnostics";
import {
    DEFAULT_FLICK_CONDITIONS,
    DEFAULT_FLICK_FEEDBACK,
    validFlickConditions,
    validFlickFeedback,
    type FlickConditions,
    type FlickFeedback,
    type FlickReport,
    type ManualFlickRates,
} from "@/js/whippyflight/flicks";
import type { FrontReport } from "@/js/whippyflight/front";
import type { GainBlobInfo } from "@/js/whippyflight/protocol";
import { DEFAULT_WEIGHTS, type Weights } from "@/js/whippyflight/weights";
import { get as getConfig, set as setConfig } from "@/js/ConfigStorage";
import { useConnectionStore } from "./connection";

const SETTINGS_KEY = "whippyflight";
const MAX_OUTPUT_LINES = 400;
const PROBE_ATTEMPTS = 3;

/** The pipeline step running, if any. */
export type PipelineStep =
    "setup" | "pull" | "import" | "erase" | "scan" | "fit" | "gen" | "front" | "flicks" | "plots" | "upload";

/**
 * Commit erases and programs the gain partition with MSP stalled; a NOR sector erase can take
 * seconds. The reply is awaited this long before the upload is reported as failed.
 */
const COMMIT_TIMEOUT_MS = 60000;

export interface WhippyFlightSettings {
    python: string;
    workdir: string;
    drag: DragMode;
    weights: Weights;
    /** Speeds per motor of the gain grid. */
    resolution: GridResolution;
    /** Flick rates set by hand; null to fly the rate profile's full-stick rates. */
    flickRates: ManualFlickRates | null;
    /** The slowest flick stick, ms; null for the fastest the setpoint target's limits allow. */
    flickRiseMs: number | null;
    /** The feedback gains the flicks fly with; those off are zeroed, to see what the rest does alone. */
    flickFeedback: FlickFeedback;
    /** What the flicks fly in besides the stick. */
    flickConditions: FlickConditions;
    /** The unit of the stored rates; settings without it hold them in degrees. */
    rateUnit: "rot";
    /** The unit of the stored σ_int; settings without it hold the low-passed rate error, rot/s. */
    integralUnit: "rot";
}

const DEFAULT_SETTINGS: WhippyFlightSettings = {
    python: "python3",
    workdir: "",
    drag: "auto",
    weights: { ...DEFAULT_WEIGHTS },
    resolution: 3,
    flickRates: null,
    flickRiseMs: null,
    flickFeedback: { ...DEFAULT_FLICK_FEEDBACK },
    flickConditions: { ...DEFAULT_FLICK_CONDITIONS, cg_shift_pct: [0, 0] },
    rateUnit: "rot",
    integralUnit: "rot",
};

/**
 * `stored` with a σ_int saved as the low-passed rate error the integral holds (rot/s) as the
 * integral state it is at that leak (rot), which weighs the same.
 */
function integralInRotations(stored: Partial<WhippyFlightSettings>): Partial<WhippyFlightSettings> {
    const weights = stored.weights as Partial<Weights> | undefined;
    if (stored.integralUnit === "rot" || typeof weights?.sigmaInt !== "number") {
        return stored;
    }
    const tauInt = typeof weights.tauInt === "number" ? weights.tauInt : DEFAULT_WEIGHTS.tauInt;
    return { ...stored, weights: { ...weights, sigmaInt: weights.sigmaInt * tauInt } as Weights };
}

/** `stored` with the rates of settings saved in degrees in rotations. */
function inRotations(stored: Partial<WhippyFlightSettings>): Partial<WhippyFlightSettings> {
    if (stored.rateUnit === "rot") {
        return stored;
    }
    const rot = (x: unknown) => (typeof x === "number" ? x / 360 : x);
    const rots = (xs: unknown) => (Array.isArray(xs) ? xs.map(rot) : xs);
    const conditions = stored.flickConditions as Record<string, unknown> | undefined;
    return {
        ...stored,
        flickRates: rots(stored.flickRates) as ManualFlickRates | null,
        flickConditions:
            conditions &&
            ({
                ...conditions,
                gyro_noise_rms: rot(conditions.gyro_noise_rms),
                turbulence_rms: rots(conditions.turbulence_rms),
            } as unknown as FlickConditions),
    };
}

function validFlickRates(rates: unknown): ManualFlickRates | null {
    return Array.isArray(rates) && rates.length === 3 && rates.every((r) => Number.isFinite(r) && r > 0)
        ? (rates as ManualFlickRates)
        : null;
}

/** The slowest flick stick the flicks command accepts, ms. */
export const MAX_FLICK_RISE_MS = 1000;

function validFlickRise(ms: unknown): number | null {
    return typeof ms === "number" && ms > 0 && ms <= MAX_FLICK_RISE_MS ? ms : null;
}

/** An upload that ended without the new gains stored; `reply` is the FC's answer when it sent one. */
export class UploadError extends Error {
    constructor(
        message: string,
        readonly reply?: UploadReply,
    ) {
        super(message);
        this.name = "UploadError";
    }
}

interface RawResponse {
    data?: DataView;
    unsupported?: number;
}

function payload(response: unknown): DataView | null {
    const r = response as RawResponse | undefined;
    return r && !r.unsupported && r.data ? r.data : null;
}

async function request(code: number, data?: number[]): Promise<UploadReply> {
    const view = payload(await MSP.promise(code, data ?? false));
    if (!view) {
        throw new UploadError(`No reply to WhippyFlight command ${code}`);
    }
    return decodeUploadReply(view);
}

/** COMMIT on the legacy path, whose request survives MSP's retries until the reply arrives. */
function commit(): Promise<UploadReply> {
    return new Promise((resolve, reject) => {
        const timer = setTimeout(
            () => reject(new UploadError("The flight controller did not confirm the write")),
            COMMIT_TIMEOUT_MS,
        );
        MSP.send_message(WhippyFlightCodes.MSP2_WHIPPYFLIGHT_UPLOAD_COMMIT, false, false, (response) => {
            clearTimeout(timer);
            const view = payload(response);
            if (view) {
                resolve(decodeUploadReply(view));
            } else {
                reject(new UploadError("No reply to the gain commit"));
            }
        });
    });
}

/**
 * WhippyFlight firmware state on the connected flight controller, and the gain upload.
 */
export const useWhippyFlightStore = defineStore("whippyflight", () => {
    const connectionStore = useConnectionStore();

    /** null until probed; false for firmware without WhippyFlight. */
    const supported = ref<boolean | null>(null);
    const status = ref<WhippyFlightStatus | null>(null);

    const uploading = ref(false);
    /** Fraction of the blob the FC has acknowledged, 0–1. */
    const uploadProgress = ref(0);
    /** True once every chunk is in and the FC is writing its dataflash. */
    const committing = ref(false);

    // Tuning pipeline (the desktop half), kept here so it survives leaving the tab.
    const env = ref<EnvReport | null>(null);
    const workspace = ref<Workspace | null>(null);
    const flightIndex = ref<FlightIndex | null>(null);
    /** Flight specs (`<log>#<n>`) the next fit uses. */
    const selectedFlights = ref<string[]>([]);
    /** Flight specs offered for selection so far; flights not in it are new, and start selected. */
    const seenFlights = new Set<string>();
    /** Diagnostics per fit id, with the `modified` stamp they were read at. */
    const fitDiagnostics = shallowRef<
        Record<string, { modified: number; diagnostics: Diagnostics; airframe: Airframe }>
    >({});
    /** The fit shown, and the one gains are computed from. */
    const activeFit = ref<string | null>(null);
    const diagnostics = computed(() => (activeFit.value && fitDiagnostics.value[activeFit.value]?.diagnostics) || null);
    /** The shown fit's airframe block: the loop rate and rate profile its logs were recorded with. */
    const airframe = computed(() => (activeFit.value && fitDiagnostics.value[activeFit.value]?.airframe) || null);
    const gains = ref<{ info: GainBlobInfo; bytes: Uint8Array } | null>(null);
    /** The gains flown through stick flicks; shallow, the traces are large. */
    const flicks = shallowRef<FlickReport | null>(null);
    /** Stick tracking (λ) as entered in the controller settings, saved or not; null without them. */
    const stickTracking = ref<number | null>(null);
    /** The motor-speed low-pass, Hz (0 for off), as entered in the controller settings, saved or not; null without them. */
    const motorLpfHz = ref<number | null>(null);
    /** Idle speed, rev/s, as entered in the controller settings, saved or not; null without them. */
    const idleRps = ref<number | null>(null);
    /** The active fit's Pareto front, once computed. */
    const front = shallowRef<FrontReport | null>(null);
    const output = ref<OutputLine[]>([]);
    const busy = ref<PipelineStep | null>(null);
    /** Fraction of the running step done, 0–1, when it can tell. */
    const progress = ref<number | null>(null);
    const error = ref("");

    function appendOutput(line: OutputLine): void {
        output.value.push(line);
        if (output.value.length > MAX_OUTPUT_LINES) {
            output.value.splice(0, output.value.length - MAX_OUTPUT_LINES);
        }
    }

    const stored = integralInRotations(
        inRotations(getConfig<Partial<WhippyFlightSettings>>(SETTINGS_KEY)[SETTINGS_KEY] ?? {}),
    );
    const settings = reactive<WhippyFlightSettings>({
        ...DEFAULT_SETTINGS,
        ...stored,
        weights: { ...DEFAULT_WEIGHTS, ...stored.weights },
        resolution: GRID_RESOLUTIONS.includes(stored.resolution as GridResolution)
            ? (stored.resolution as GridResolution)
            : DEFAULT_SETTINGS.resolution,
        flickRates: validFlickRates(stored.flickRates),
        flickRiseMs: validFlickRise(stored.flickRiseMs),
        flickFeedback: validFlickFeedback(
            stored.flickFeedback,
            (stored as { flickFeedforwardOnly?: unknown }).flickFeedforwardOnly,
        ),
        flickConditions: validFlickConditions(stored.flickConditions),
        rateUnit: "rot",
        integralUnit: "rot",
    });
    watch(settings, () => setConfig({ [SETTINGS_KEY]: { ...settings } }), { deep: true });

    watch(
        () => connectionStore.connectionValid,
        (valid) => {
            if (!valid) {
                supported.value = null;
                status.value = null;
                stickTracking.value = null;
                motorLpfHz.value = null;
                idleRps.value = null;
            }
        },
    );

    /**
     * Read the gain status.
     * @returns the status, or null when the firmware does not speak WhippyFlight
     */
    async function refreshStatus(): Promise<WhippyFlightStatus | null> {
        const view = payload(await MSP.promise(WhippyFlightCodes.MSP2_WHIPPYFLIGHT_STATUS));
        status.value = view ? decodeStatus(view) : null;
        return status.value;
    }

    /**
     * The probe runs as the connection opens, while the first tab's switch clears the MSP queue;
     * a request cancelled that way is sent again.
     * @returns whether the connected firmware is WhippyFlight
     */
    async function probe(): Promise<boolean> {
        for (let attempt = 1; ; attempt++) {
            try {
                supported.value = (await refreshStatus()) !== null;
                return supported.value;
            } catch (error) {
                const cleared = error instanceof MspCancelledError && error.reason === "cleanup";
                if (!cleared || attempt >= PROBE_ATTEMPTS) {
                    console.warn("WhippyFlight probe failed:", error);
                    supported.value = false;
                    return false;
                }
            }
        }
    }

    /**
     * Send a gain blob and have the FC validate, store and load it.
     * @throws {UploadError} when the FC refused or the gains were not stored
     */
    async function upload(blob: Uint8Array): Promise<void> {
        if (uploading.value) {
            throw new UploadError("An upload is already in progress");
        }
        uploading.value = true;
        committing.value = false;
        uploadProgress.value = 0;
        connectionStore.pauseLiveData();
        let begun = false;
        try {
            await connectionStore.clearMspQueue();
            const before = await refreshStatus();
            if (!before) {
                throw new UploadError("The flight controller does not run WhippyFlight");
            }
            const chunkSize = before.maxChunk;

            const begin = await request(
                WhippyFlightCodes.MSP2_WHIPPYFLIGHT_UPLOAD_BEGIN,
                encodeUploadBegin(blob.length),
            );
            if (begin.result !== UploadResult.OK) {
                throw new UploadError("Upload refused", begin);
            }
            begun = true;

            for (let offset = 0; offset < blob.length; offset += chunkSize) {
                const chunk = blob.subarray(offset, offset + chunkSize);
                const reply = await request(
                    WhippyFlightCodes.MSP2_WHIPPYFLIGHT_UPLOAD_DATA,
                    encodeUploadData(offset, chunk),
                );
                // A retried chunk the FC already took is answered out of order; its count says so.
                const acknowledged =
                    reply.result === UploadResult.OK ||
                    (reply.result === UploadResult.OUT_OF_ORDER &&
                        (await refreshStatus())?.uploadReceived === offset + chunk.length);
                if (!acknowledged) {
                    throw new UploadError("Chunk refused", reply);
                }
                uploadProgress.value = (offset + chunk.length) / blob.length;
            }

            committing.value = true;
            const result = await commit();
            begun = false;
            if (result.result !== UploadResult.OK) {
                throw new UploadError("Gains not stored", result);
            }
        } catch (error) {
            if (begun && connectionStore.connectionValid) {
                await MSP.promise(WhippyFlightCodes.MSP2_WHIPPYFLIGHT_UPLOAD_ABORT).catch(() => undefined);
            }
            throw error;
        } finally {
            uploading.value = false;
            committing.value = false;
            connectionStore.resumeLiveData();
            if (connectionStore.connectionValid) {
                await refreshStatus().catch(() => undefined);
            }
        }
    }

    return {
        supported,
        status,
        settings,
        env,
        workspace,
        flightIndex,
        selectedFlights,
        seenFlights,
        fitDiagnostics,
        airframe,
        activeFit,
        diagnostics,
        gains,
        flicks,
        stickTracking,
        motorLpfHz,
        idleRps,
        front,
        output,
        busy,
        progress,
        error,
        appendOutput,
        uploading,
        uploadProgress,
        committing,
        probe,
        refreshStatus,
        upload,
    };
});

export { GainLoadResult, GainSource, UploadResult };
