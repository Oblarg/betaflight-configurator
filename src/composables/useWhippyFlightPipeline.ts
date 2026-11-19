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

import { computed, watch, type ComputedRef } from "vue";
import MSP from "@/js/msp";
import MSPCodes from "@/js/msp/MSPCodes";
import { i18n } from "@/js/localization";
import { isTauriDesktop } from "@/js/utils/checkCompatibility";
import {
    CLI_DUMP_FILE,
    GAINS_FILE,
    cancelJob,
    checkEnv,
    deleteFit,
    deleteLog,
    flyFlicks,
    pickWorkdir,
    readDiagnostics,
    readFile,
    readFlicks,
    readFront,
    readAirframe,
    scoreFront,
    applyFrontPoint,
    importFiles,
    pickFiles,
    readFlightIndex,
    readWorkspace,
    reveal,
    runJob,
    writeFile,
    type Grid,
    type Job,
    type Toolchain,
} from "@/js/whippyflight/pipeline";
import {
    logMaxRatesRps,
    maxRatesRps,
    type FlickRatesSource,
    type MaxRates,
    type RatesSource,
} from "@/js/whippyflight/flicks";
import { derivedSigmaRate, derivedTauDeriv, frontWeights } from "@/js/whippyflight/front";
import RateCurve from "@/js/RateCurve";
import { fitId, listFlights, specOfLabel, unscannedLogs, type Flight } from "@/js/whippyflight/flights";
import { parseGainBlob } from "@/js/whippyflight/protocol";
import { UploadResult, GainLoadResult } from "@/js/msp/WhippyFlightCodes";
import { useFlightControllerStore } from "@/stores/fc";
import { useConnectionStore } from "@/stores/connection";
import { UploadError, useWhippyFlightStore, type PipelineStep } from "@/stores/whippyflight";
import { useDataflashPull } from "./useDataflashPull";
import { useDataflashErase } from "./useDataflashErase";
import { send as sendCli, isMspCliSupported } from "./useMspCliSession";

const DUMP_TIMEOUT_MS = 10000;

/** The steps of the tuning flow, in order. */
export type FlowStep = "setup" | "logs" | "fit" | "gains" | "upload";

/** `<craft>_<yyyymmdd>_<hhmmss>.bbl`, a plain file name the desktop side accepts. */
export function logFileName(craft: string, date: Date): string {
    const pad = (n: number) => String(n).padStart(2, "0");
    const stamp =
        `${date.getFullYear()}${pad(date.getMonth() + 1)}${pad(date.getDate())}_` +
        `${pad(date.getHours())}${pad(date.getMinutes())}${pad(date.getSeconds())}`;
    const safe = craft.replaceAll(/[^A-Za-z0-9_-]+/g, "_").replaceAll(/^_+|_+$/g, "") || "fc";
    return `${safe}_${stamp}.bbl`;
}

/** @returns the error's message, for the tab's error line */
export function describeError(error: unknown): string {
    if (error instanceof UploadError && error.reply) {
        const reason = i18n.getMessage(`whippyflightUpload_${UploadResult[error.reply.result]}`);
        return error.reply.result === UploadResult.REJECTED
            ? `${reason} ${i18n.getMessage(`whippyflightGain_${GainLoadResult[error.reply.gainResult]}`)}`
            : reason;
    }
    if (error instanceof Error) {
        return error.message;
    }
    return typeof error === "string" ? error : JSON.stringify(error);
}

export interface WhippyFlightPipeline {
    isDesktop: boolean;
    toolchainReady: ComputedRef<boolean>;
    flashUsed: ComputedRef<number>;
    currentStep: ComputedRef<FlowStep>;
    /** Every scanned flight in the working directory. */
    flights: ComputedRef<Flight[]>;
    /** Logs not scanned yet (new, or changed since). */
    unscanned: ComputedRef<string[]>;
    chooseWorkdir: () => Promise<void>;
    checkEnvironment: () => Promise<void>;
    refreshWorkspace: () => Promise<void>;
    refreshFlash: () => Promise<void>;
    pullLogs: () => Promise<void>;
    /** Pick blackbox logs (or, with `dump`, a CLI dump) to copy into the working directory. */
    importLogs: (dump: boolean) => Promise<void>;
    /** The loop rate gains would be computed for, and whether it's the FC's or the logs'. */
    gainsLoop: ComputedRef<{ dt: number; from: RatesSource } | null>;
    /** The grid gains would be computed over; its idle speed the FC's (as entered), else the logs'. */
    gainsGrid: ComputedRef<Grid>;
    eraseFlash: () => Promise<void>;
    removeLog: (name: string) => Promise<void>;
    scanLogs: () => Promise<void>;
    /** Fit the selected flights together. */
    runFit: () => Promise<void>;
    /** Fit each of `specs` on its own, one after another. */
    runFitEach: (specs: string[]) => Promise<void>;
    selectFit: (id: string) => Promise<void>;
    removeFit: (id: string) => Promise<void>;
    /** Compute gains from the shown fit, and fly them through stick flicks. */
    runGains: () => Promise<void>;
    /** Fly the current gains through stick flicks (gains computed before flicks existed). */
    runFlicks: () => Promise<void>;
    /** Compute gains over a grid of LQR weights from the shown fit, and score each. */
    exploreFront: () => Promise<void>;
    /** Make point `index` of the shown front the gains, and fly them through stick flicks. */
    pickFrontPoint: (index: number) => Promise<void>;
    uploadGains: () => Promise<void>;
    cancel: () => Promise<void>;
    revealWorkdir: (sub?: string) => Promise<void>;
    /** Write PNGs of the shown fit's diagnostics, or of the flicks, and open their directory. */
    savePlots: (what: "fit" | "gains") => Promise<void>;
}

/**
 * The WhippyFlight tuning flow: pull flight logs off the FC (or import them) into a working
 * directory, fit the plant model, compute gains for the FC's loop rate (or the logs', with no FC),
 * and upload them.
 */
export function useWhippyFlightPipeline(): WhippyFlightPipeline {
    const store = useWhippyFlightStore();
    const fcStore = useFlightControllerStore();
    const connectionStore = useConnectionStore();
    const dataflashPull = useDataflashPull();
    const isDesktop = isTauriDesktop();

    let eraseDone: { resolve: () => void; reject: (error: unknown) => void } | null = null;
    const dataflashErase = useDataflashErase({
        onComplete: () => eraseDone?.resolve(),
        onError: (error) => eraseDone?.reject(error),
        onFinish: (reason) => {
            if (reason !== "complete" && reason !== "error") {
                eraseDone?.reject(new Error(reason));
            }
        },
    });

    watch(dataflashPull.progress, (percent) => {
        if (store.busy === "pull") {
            store.progress = percent / 100;
        }
    });

    const toolchain = (): Toolchain => ({ python: store.settings.python });

    const toolchainReady = computed(() => !!store.env?.pythonVersion && !store.env.missingModules.length);

    const flashUsed = computed(() => fcStore.dataflash?.usedSize ?? 0);

    const flights = computed(() => listFlights(store.flightIndex, store.workspace?.logs ?? []));
    const unscanned = computed(() => unscannedLogs(store.flightIndex, store.workspace?.logs ?? []));

    const currentStep = computed<FlowStep>(() => {
        const ws = store.workspace;
        if (!toolchainReady.value || !store.settings.workdir || !ws) {
            return "setup";
        }
        if (!flights.value.some((flight) => flight.usable)) {
            return "logs";
        }
        if (!store.diagnostics) {
            return "fit";
        }
        if (!store.gains || ws.gainsFit !== store.activeFit) {
            return "gains";
        }
        return "upload";
    });

    /** Run one step: owns `busy`, `progress` and `error` for its duration. */
    async function step(name: PipelineStep, body: () => Promise<void>): Promise<void> {
        if (store.busy) {
            return;
        }
        store.busy = name;
        store.progress = null;
        store.error = "";
        try {
            await body();
        } catch (error) {
            store.error = describeError(error);
        } finally {
            store.busy = null;
            store.progress = null;
        }
    }

    async function refreshWorkspace(): Promise<void> {
        if (!isDesktop || !store.settings.workdir) {
            store.workspace = null;
            return;
        }
        try {
            const ws = await readWorkspace(store.settings.workdir);
            store.workspace = ws;
            store.flightIndex = ws.hasFlightIndex ? await readFlightIndex(store.settings.workdir) : null;
            updateSelection();
            await loadFitDiagnostics();
            await loadFront();
            await loadGains(ws.hasGains);
            if (!ws.hasFlicks) {
                store.flicks = null;
            } else if (!store.flicks) {
                store.flicks = await readFlicks(store.settings.workdir);
            }
        } catch (error) {
            store.workspace = null;
            store.error = describeError(error);
        }
    }

    /** Drop selected flights that are gone or unusable; select usable flights not offered before. */
    function updateSelection(): void {
        const usable = flights.value.filter((flight) => flight.usable).map((flight) => flight.spec);
        const fresh = usable.filter((spec) => !store.seenFlights.has(spec));
        fresh.forEach((spec) => store.seenFlights.add(spec));
        store.selectedFlights = [...store.selectedFlights.filter((spec) => usable.includes(spec)), ...fresh];
    }

    /** Read the diagnostics of fits that are new or rewritten, and keep `activeFit` on one that exists. */
    async function loadFitDiagnostics(): Promise<void> {
        const fits = store.workspace?.fits ?? [];
        const cache: typeof store.fitDiagnostics = {};
        for (const fit of fits.filter((f) => f.hasDiagnostics)) {
            const known = store.fitDiagnostics[fit.id];
            cache[fit.id] =
                known?.modified === fit.modified
                    ? known
                    : {
                          modified: fit.modified,
                          diagnostics: await readDiagnostics(store.settings.workdir, fit.id),
                          airframe: fit.hasCoefficients
                              ? await readAirframe(store.settings.workdir, fit.id).catch(() => ({}))
                              : {},
                      };
        }
        store.fitDiagnostics = cache;
        if (!store.activeFit || !cache[store.activeFit]) {
            const gainsFit = store.workspace?.gainsFit;
            const newest = fits.filter((f) => cache[f.id]).sort((a, b) => b.modified - a.modified)[0];
            store.activeFit = gainsFit && cache[gainsFit] ? gainsFit : (newest?.id ?? null);
        }
    }

    /** Keep `front` on the active fit's, if it has one. */
    async function loadFront(): Promise<void> {
        const fit = store.activeFit;
        const entry = store.workspace?.fits.find((f) => f.id === fit);
        if (!fit || !entry?.hasFront) {
            store.front = null;
        } else if (store.front?.fit !== fit) {
            store.front = await readFront(store.settings.workdir, fit).catch(() => null);
        }
    }

    async function loadGains(present: boolean): Promise<void> {
        if (!present) {
            store.gains = null;
            return;
        }
        const bytes = await readFile(store.settings.workdir, GAINS_FILE);
        const info = parseGainBlob(bytes);
        if ("error" in info) {
            store.gains = null;
            throw new Error(i18n.getMessage(`whippyflightBlob_${info.error}`));
        }
        store.gains = { info, bytes };
    }

    async function checkEnvironment(): Promise<void> {
        if (!isDesktop) {
            return;
        }
        store.env = await checkEnv(toolchain());
    }

    async function chooseWorkdir(): Promise<void> {
        const dir = await pickWorkdir(store.settings.workdir || undefined);
        if (dir) {
            store.settings.workdir = dir;
            store.selectedFlights = [];
            store.seenFlights.clear();
            store.fitDiagnostics = {};
            store.activeFit = null;
            await refreshWorkspace();
            await scanLogs();
        }
    }

    async function refreshFlash(): Promise<void> {
        await MSP.promise(MSPCodes.MSP_DATAFLASH_SUMMARY).catch(() => undefined);
    }

    async function pullLogs(): Promise<void> {
        await step("pull", async () => {
            const bytes = await dataflashPull.pull();
            const name = logFileName(fcStore.config.craftName || fcStore.config.name || "", new Date());
            await writeFile(store.settings.workdir, `logs/${name}`, bytes);
            if (isMspCliSupported()) {
                // fit_model checks the motor layout against a dump; `diff` omits the default mixer.
                const dump = await sendCli("dump", { timeoutMs: DUMP_TIMEOUT_MS }).catch(() => null);
                if (dump?.length) {
                    await writeFile(store.settings.workdir, CLI_DUMP_FILE, new TextEncoder().encode(dump.join("\n")));
                }
            }
            await refreshWorkspace();
        });
        await scanLogs();
    }

    /** Copy blackbox logs, or a CLI dump, from anywhere into the working directory. */
    async function importLogs(dump: boolean): Promise<void> {
        const paths = await pickFiles(dump);
        if (!paths.length) {
            return;
        }
        await step("import", async () => {
            await importFiles(store.settings.workdir, paths, dump);
            await refreshWorkspace();
        });
        await scanLogs();
    }

    function eraseFlash(): Promise<void> {
        return step("erase", async () => {
            await new Promise<void>((resolve, reject) => {
                eraseDone = { resolve, reject };
                void dataflashErase.start();
            }).finally(() => {
                eraseDone = null;
            });
            await refreshFlash();
        });
    }

    async function removeLog(name: string): Promise<void> {
        await deleteLog(store.settings.workdir, name);
        await refreshWorkspace();
    }

    async function run(job: Job): Promise<void> {
        const code = await runJob(toolchain(), store.settings.workdir, job, store.appendOutput);
        if (code !== 0) {
            const last = store.output.filter((line) => line.stderr).at(-1)?.line;
            throw new Error(
                i18n.getMessage("whippyflightScriptFailed", {
                    code,
                    detail: last ?? "",
                    interpolation: { escapeValue: false },
                }),
            );
        }
    }

    /** Index logs that are new or changed; a no-op when every log is scanned or Python isn't set up. */
    function scanLogs(): Promise<void> {
        if (!unscanned.value.length || !toolchainReady.value) {
            return Promise.resolve();
        }
        return step("scan", async () => {
            store.output = [];
            await run({ kind: "scan", logs: unscanned.value });
            await refreshWorkspace();
        });
    }

    function fitJob(specs: string[]): Job & { kind: "fit" } {
        return {
            kind: "fit",
            flights: specs,
            drag: store.settings.drag,
            fit: fitId(specs, store.settings.drag),
        };
    }

    function runFit(): Promise<void> {
        return step("fit", async () => {
            store.output = [];
            const job = fitJob(store.selectedFlights);
            try {
                await run(job);
            } finally {
                await refreshWorkspace();
            }
            store.activeFit = job.fit;
        });
    }

    function runFitEach(specs: string[]): Promise<void> {
        return step("fit", async () => {
            store.output = [];
            for (const [i, spec] of specs.entries()) {
                store.progress = i / specs.length;
                const job = fitJob([spec]);
                store.appendOutput({ line: `── ${spec}`, stderr: false });
                try {
                    await run(job);
                } finally {
                    await refreshWorkspace();
                }
                // Show the first result straight away, then leave the view to the user.
                if (i === 0) {
                    store.activeFit = job.fit;
                }
            }
        });
    }

    async function selectFit(id: string): Promise<void> {
        if (!store.fitDiagnostics[id]) {
            await refreshWorkspace();
        }
        if (store.fitDiagnostics[id]) {
            store.activeFit = id;
            await loadFront();
        }
    }

    async function removeFit(id: string): Promise<void> {
        await deleteFit(store.settings.workdir, id).catch((error) => (store.error = describeError(error)));
        if (store.activeFit === id) {
            store.activeFit = null;
        }
        await refreshWorkspace();
    }

    /** The PID loop the gains are computed for: the connected FC's, else the one the fit's logs ran at. */
    const gainsLoop = computed<{ dt: number; from: RatesSource } | null>(() => {
        if (connectionStore.connectionValid && store.status?.pidDt) {
            return { dt: store.status.pidDt, from: "fc" };
        }
        const hz = store.airframe?.loop_hz;
        return hz ? { dt: 1 / hz, from: "log" } : null;
    });

    const gainsGrid = computed<Grid>(() => ({
        resolution: store.settings.resolution,
        idleRps: (connectionStore.connectionValid ? store.idleRps : null) ?? store.airframe?.idle_rps ?? 0,
    }));

    /** The shown fit and the loop to compute its gains for, refreshing the FC's first. */
    async function gainsTarget(): Promise<{ fit: string; dt: number | null }> {
        const fit = store.activeFit;
        if (!fit) {
            throw new Error(i18n.getMessage("whippyflightNoFitChosen"));
        }
        if (connectionStore.connectionValid) {
            await store.refreshStatus().catch(() => undefined);
        }
        const loop = gainsLoop.value;
        if (!loop) {
            throw new Error(i18n.getMessage("whippyflightNoLoopRate"));
        }
        return { fit, dt: loop.from === "fc" ? loop.dt : null };
    }

    function runGains(): Promise<void> {
        return step("gen", async () => {
            const { fit, dt } = await gainsTarget();
            store.gains = null;
            store.flicks = null;
            store.output = [];
            await run({ kind: "gen", dt, fit, weights: { ...store.settings.weights }, grid: { ...gainsGrid.value } });
            await refreshWorkspace();
            await simulateFlicks();
        });
    }

    function exploreFront(): Promise<void> {
        return step("front", async () => {
            const { fit, dt } = await gainsTarget();
            const weights = store.settings.weights;
            const prior = store.front?.fit === fit ? store.front : null;
            const loopDt = gainsLoop.value?.dt ?? null;
            // σ_rate and τ_deriv are derived again when the settings they were derived at change;
            // one set by hand (not the last derived) is kept, else it follows the derived one.
            const derivedRate = prior ? derivedSigmaRate(prior, weights, gainsGrid.value, loopDt) : null;
            const rateByHand = prior?.derived_sigma_rate != null && weights.sigmaRate !== prior.derived_sigma_rate;
            const sigmaRate = derivedRate !== null || rateByHand ? weights.sigmaRate : null;
            const derivedTau =
                prior && sigmaRate !== null ? derivedTauDeriv(prior, weights, gainsGrid.value, loopDt) : null;
            const tauByHand = prior?.derived_tau_deriv != null && weights.tauDeriv !== prior.derived_tau_deriv;
            const tauDeriv = derivedTau !== null || tauByHand ? weights.tauDeriv : null;
            store.front = null;
            store.output = [];
            // Roughness needs the fit's sensor noise.
            await extractNoise(fit);
            await run({
                kind: "front",
                dt,
                fit,
                sigmaRate,
                derivedSigmaRate: derivedRate,
                tauInt: weights.tauInt,
                tauDeriv,
                derivedTauDeriv: derivedTau,
                grid: { ...gainsGrid.value },
            });
            const front = await scoreFront(store.settings.workdir, fit);
            store.front = front;
            store.settings.weights = {
                ...store.settings.weights,
                sigmaRate: sigmaRate ?? front.sigma_rate,
                tauDeriv: tauDeriv ?? front.tau_deriv,
            };
            await refreshWorkspace();
        });
    }

    function pickFrontPoint(index: number): Promise<void> {
        return step("gen", async () => {
            const front = store.front;
            const point = front?.points[index];
            if (!front || !point) {
                return;
            }
            await applyFrontPoint(store.settings.workdir, front.fit, index);
            store.settings.weights = frontWeights(front, point);
            store.gains = null;
            store.flicks = null;
            await refreshWorkspace();
            await simulateFlicks();
        });
    }

    /**
     * The flicks' rates: those set by hand, else the full-stick rates of the connected FC, else
     * those of the logs the gains' fit came from.
     */
    async function maxRates(): Promise<{ rps: MaxRates; from: FlickRatesSource } | null> {
        const manual = store.settings.flickRates;
        if (manual) {
            return { rps: [...manual] as MaxRates, from: "manual" };
        }
        if (connectionStore.connectionValid) {
            await MSP.promise(MSPCodes.MSP_RC_TUNING);
            return { rps: maxRatesRps(new RateCurve(false)), from: "fc" };
        }
        const fit = store.workspace?.gainsFit;
        const rates = fit ? store.fitDiagnostics[fit]?.airframe.stick_rates : null;
        return rates ? { rps: logMaxRatesRps(rates), from: "log" } : null;
    }

    /** Extract a fit's sensor noise from its longest flight, if it hasn't been. */
    async function extractNoise(fit: string | null | undefined): Promise<void> {
        const entry = store.workspace?.fits.find((f) => f.id === fit);
        const flights = fit ? store.fitDiagnostics[fit]?.diagnostics.flights : undefined;
        if (!fit || !entry || entry.hasNoise || !flights?.length || !toolchainReady.value) {
            return;
        }
        const longest = flights.reduce((a, b) => (b.kept_s > a.kept_s ? b : a));
        try {
            await run({ kind: "noise", fit, flight: specOfLabel(longest.label) });
        } catch {
            // Without noise the flicks just lack a roughness figure; the output says why.
        }
    }

    async function simulateFlicks(): Promise<void> {
        await extractNoise(store.workspace?.gainsFit);
        const rates = await maxRates();
        store.flicks = await flyFlicks(
            store.settings.workdir,
            rates,
            store.stickTracking,
            store.motorLpfHz,
            store.settings.flickRiseMs,
            store.settings.flickFeedback,
            store.settings.flickConditions,
        );
        await refreshWorkspace();
    }

    function runFlicks(): Promise<void> {
        return step("flicks", simulateFlicks);
    }

    function uploadGains(): Promise<void> {
        return step("upload", async () => {
            if (!store.gains) {
                throw new Error(i18n.getMessage("whippyflightNoGains"));
            }
            const stop = watch(
                () => store.uploadProgress,
                (p) => (store.progress = p),
            );
            try {
                await store.upload(store.gains.bytes);
            } finally {
                stop();
            }
        });
    }

    async function cancel(): Promise<void> {
        if (["scan", "fit", "gen", "front", "flicks", "plots"].includes(store.busy ?? "")) {
            await cancelJob();
        } else if (store.busy === "erase") {
            dataflashErase.cancel();
        }
    }

    async function revealWorkdir(sub?: string): Promise<void> {
        await reveal(store.settings.workdir, sub).catch((error) => (store.error = describeError(error)));
    }

    function savePlots(what: "fit" | "gains"): Promise<void> {
        return step("plots", async () => {
            const fit = store.activeFit;
            if (what === "fit" && !fit) {
                throw new Error(i18n.getMessage("whippyflightNoFitChosen"));
            }
            store.output = [];
            if (what === "fit") {
                await run({ kind: "fitPlots", fit: fit! });
            } else {
                // The flick plots' directory holds the others: they go first, as they clear it.
                if (store.workspace?.hasFlicks) {
                    await run({ kind: "flickPlots" });
                }
                await run({ kind: "gainPlots" });
                if (store.workspace?.gainsFit) {
                    await run({ kind: "occupancyPlots" });
                }
            }
            await refreshWorkspace();
            await reveal(store.settings.workdir, what === "fit" ? `fits/${fit}/plots` : "gains/plots");
        });
    }

    return {
        isDesktop,
        toolchainReady,
        flashUsed,
        currentStep,
        flights,
        unscanned,
        chooseWorkdir,
        checkEnvironment,
        refreshWorkspace,
        refreshFlash,
        pullLogs,
        importLogs,
        gainsLoop,
        gainsGrid,
        eraseFlash,
        removeLog,
        scanLogs,
        runFit,
        runFitEach,
        selectFit,
        removeFit,
        runGains,
        runFlicks,
        exploreFront,
        pickFrontPoint,
        uploadGains,
        cancel,
        revealWorkdir,
        savePlots,
    };
}
