import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { nextTick } from "vue";
import type { Diagnostics } from "../../src/js/whippyflight/diagnostics";
import type { Airframe, FlightIndex, Job, Workspace } from "../../src/js/whippyflight/pipeline";
import { DEFAULT_WEIGHTS, type Weights } from "../../src/js/whippyflight/weights";
import type { FrontPoint, FrontReport } from "../../src/js/whippyflight/front";

/** An in-memory working directory; jobs behave like the scripts they stand for. */
const { disk } = vi.hoisted(() => ({
    disk: {
        logs: [] as { name: string; size: number; flights: { duration_s: number; kept_s?: number }[] }[],
        index: null as FlightIndex | null,
        fits: new Map<string, { flights: string[]; modified: number }>(),
        /** What each fit's logs record in their headers. */
        airframes: {} as Record<string, Airframe>,
        /** Fits whose sensor noise has been extracted. */
        noise: new Set<string>(),
        gainsFit: null as string | null,
        gainsWeights: null as Weights | null,
        flicks: null as string | null,
        /** Each fit's scored Pareto front. */
        fronts: new Map<string, FrontReport>(),
        clock: 1,
        jobs: [] as Job[],
        fail: new Set<string>(),
    },
}));

vi.mock("../../src/js/msp", () => ({ default: { promise: vi.fn(), send_message: vi.fn() } }));
vi.mock("../../src/js/utils/checkCompatibility", async (importOriginal) => ({
    ...(await importOriginal<object>()),
    isTauriDesktop: () => true,
}));
vi.mock("../../src/composables/useDataflashPull", () => ({
    useDataflashPull: () => ({ progress: { value: 0 }, pull: vi.fn() }),
}));
vi.mock("../../src/composables/useDataflashErase", () => ({
    useDataflashErase: () => ({ start: vi.fn(), cancel: vi.fn() }),
}));
vi.mock("../../src/composables/useMspCliSession", () => ({ send: vi.fn(), isMspCliSupported: () => false }));
vi.mock("../../src/js/whippyflight/pipeline", async (importOriginal) => {
    const actual = await importOriginal<typeof import("../../src/js/whippyflight/pipeline")>();
    const diagnostics = (flights: string[]): Diagnostics =>
        ({
            heldout_method: "x",
            kept_s: 1,
            axes: {},
            warnings: [],
            flights: flights.map((label, i) => ({ label: `logs/${label}`, kept_s: 10 + i })),
        }) as unknown as Diagnostics;
    return {
        ...actual,
        checkEnv: vi.fn(),
        readWorkspace: vi.fn(async (): Promise<Workspace> => ({
            logs: disk.logs.map(({ name, size }) => ({ name, size })),
            hasFlightIndex: !!disk.index,
            hasCliDump: false,
            fits: [...disk.fits].map(([id, fit]) => ({
                id,
                hasCoefficients: true,
                hasDiagnostics: true,
                hasNoise: disk.noise.has(id),
                noiseRms: disk.noise.has(id) ? { gyro: 0.5, motor: 40, motor_speed_exponent: null } : null,
                turbulence: null,
                latency: null,
                hasFront: disk.fronts.has(id),
                plots: [],
                modified: fit.modified,
            })),
            hasGains: !!disk.gainsFit,
            gainsFit: disk.gainsFit,
            gainsWeights: disk.gainsWeights,
            hasFlicks: !!disk.flicks,
            flickPlots: [],
        })),
        readFlightIndex: vi.fn(async () => structuredClone(disk.index)),
        readFile: vi.fn(async () => (await import("./whippyflight_fixtures")).makeBlob()),
        readDiagnostics: vi.fn(async (_dir: string, id: string) => diagnostics(disk.fits.get(id)!.flights)),
        readAirframe: vi.fn(async (_dir: string, id: string) => disk.airframes[id] ?? {}),
        runJob: vi.fn(async (_toolchain: unknown, _dir: string, job: Job) => {
            disk.jobs.push(job);
            if (job.kind === "scan") {
                disk.index ??= {};
                for (const name of job.logs) {
                    const log = disk.logs.find((l) => l.name === name)!;
                    disk.index[name] = {
                        size: log.size,
                        flights: log.flights.map((f, i) => ({
                            index: i + 1,
                            ...f,
                            reason: f.kept_s === undefined ? "short" : null,
                        })),
                    };
                }
            } else if (job.kind === "fit") {
                if (disk.fail.has(job.fit)) {
                    return 1;
                }
                disk.fits.set(job.fit, { flights: job.flights, modified: disk.clock++ });
            } else if (job.kind === "noise") {
                disk.noise.add(job.fit);
            } else if (job.kind === "gen") {
                disk.gainsFit = job.fit;
                disk.gainsWeights = job.weights;
                disk.flicks = null;
            }
            return 0;
        }),
        flyFlicks: vi.fn(async (_dir: string, _rates: unknown) => {
            disk.flicks = disk.gainsFit;
            return { fit: disk.gainsFit, flicks: [] };
        }),
        readFlicks: vi.fn(async () => ({ fit: disk.flicks, flicks: [] })),
        scoreFront: vi.fn(async (_dir: string, fit: string): Promise<FrontReport> => {
            const job = [...disk.jobs].reverse().find((j: Job) => j.kind === "front" && j.fit === fit) as Job & {
                kind: "front";
            };
            const point = (sigma_deriv: number, sigma_int: number, grid: [number, number]): FrontPoint => ({
                sigma_deriv,
                sigma_int,
                grid,
                kick_iae_ms: 4 * sigma_deriv,
                rough_v: 0.05 / sigma_deriv,
                delay_margin_ms: 3,
                trim_deg: 100 * sigma_int,
            });
            const derivedRate = job.derivedSigmaRate ?? 2;
            const rate = job.sigmaRate ?? derivedRate;
            const derived = job.derivedTauDeriv ?? 0.0075 * rate;
            const report = {
                version: 2,
                fit,
                dt: job.dt ?? 1 / 4000,
                sigma_rate: rate,
                derived_sigma_rate: derivedRate,
                tau_deriv: job.tauDeriv ?? derived,
                tau_int: job.tauInt,
                derived_tau_deriv: derived,
                resolution: job.grid.resolution,
                idle_rps: job.grid.idleRps,
                gyro_filters: true,
                max_margin_ms: 16,
                bf_rough_v: null,
                points: [point(15, 0.06, [0, 0]), point(10, 0.02, [1, 1])],
            };
            disk.fronts.set(fit, report);
            return structuredClone(report);
        }),
        readFront: vi.fn(async (_dir: string, fit: string) => structuredClone(disk.fronts.get(fit)!)),
        applyFrontPoint: vi.fn(async (_dir: string, fit: string, index: number) => {
            const front = disk.fronts.get(fit)!;
            const p = front.points[index];
            disk.gainsFit = fit;
            disk.gainsWeights = {
                sigmaRate: front.sigma_rate,
                sigmaDeriv: p.sigma_deriv,
                sigmaInt: p.sigma_int,
                tauDeriv: front.tau_deriv,
                tauInt: front.tau_int,
            };
            disk.flicks = null;
        }),
        deleteFit: vi.fn(async (_dir: string, id: string) => void disk.fits.delete(id)),
        reveal: vi.fn(async () => undefined),
        pickFiles: vi.fn(async (): Promise<string[]> => []),
        importFiles: vi.fn(async (_dir: string, paths: string[], dump: boolean) => {
            if (dump) {
                return ["cli_dump.txt"];
            }
            const names = paths.map((p) => p.split("/").at(-1)!);
            disk.logs.push(...names.map((name) => ({ name, size: 30, flights: [{ duration_s: 40, kept_s: 35 }] })));
            return names;
        }),
    };
});

import { useWhippyFlightPipeline } from "../../src/composables/useWhippyFlightPipeline";
import { useWhippyFlightStore } from "../../src/stores/whippyflight";
import { useConnectionStore } from "../../src/stores/connection";
import * as pipelineIo from "../../src/js/whippyflight/pipeline";

function setup() {
    const store = useWhippyFlightStore();
    store.settings.workdir = "/w";
    store.env = { pythonVersion: "3.12", missingModules: [], hasMatplotlib: false };
    return { store, pipeline: useWhippyFlightPipeline() };
}

describe("WhippyFlight stored settings", () => {
    beforeEach(() => setActivePinia(createPinia()));

    it("reads a σ_int saved as a rate error as the integral state it is", async () => {
        const weights = { ...DEFAULT_WEIGHTS, sigmaInt: 0.012, tauInt: 3 };
        localStorage.setItem("whippyflight", JSON.stringify({ whippyflight: { rateUnit: "rot", weights } }));
        const store = useWhippyFlightStore();
        expect(store.settings.weights.sigmaInt).toBeCloseTo(0.036, 12);
        // Once saved in rotations it stays.
        store.settings.workdir = "/w";
        await nextTick();
        setActivePinia(createPinia());
        expect(useWhippyFlightStore().settings.weights.sigmaInt).toBeCloseTo(0.036, 12);
        localStorage.removeItem("whippyflight");
    });
});

describe("WhippyFlight pipeline flight selection", () => {
    beforeEach(() => {
        setActivePinia(createPinia());
        Object.assign(disk, {
            logs: [],
            index: null,
            fits: new Map(),
            airframes: {},
            noise: new Set(),
            gainsFit: null,
            gainsWeights: null,
            flicks: null,
            fronts: new Map(),
            clock: 1,
            jobs: [],
        });
        vi.mocked(pipelineIo.flyFlicks).mockClear();
        disk.fail.clear();
        disk.logs.push({ name: "a.bbl", size: 10, flights: [{ duration_s: 0.5 }, { duration_s: 60, kept_s: 50 }] });
    });

    it("scans new logs and selects their usable flights", async () => {
        const { store, pipeline } = setup();
        await pipeline.refreshWorkspace();
        expect(pipeline.unscanned.value).toEqual(["a.bbl"]);
        await pipeline.scanLogs();
        expect(store.selectedFlights).toEqual(["a.bbl#2"]);

        // A deselected flight stays deselected; a new log's flights arrive selected.
        store.selectedFlights = [];
        disk.logs.push({ name: "b.bbl", size: 20, flights: [{ duration_s: 30, kept_s: 25 }] });
        await pipeline.refreshWorkspace();
        await pipeline.scanLogs();
        expect(disk.jobs.filter((j) => j.kind === "scan").map((j) => (j as { logs: string[] }).logs)).toEqual([
            ["a.bbl"],
            ["b.bbl"],
        ]);
        expect(store.selectedFlights).toEqual(["b.bbl#1"]);
        expect(pipeline.currentStep.value).toBe("fit");
    });

    it("keeps one fit per selection and computes gains from the one shown", async () => {
        const { store, pipeline } = setup();
        disk.logs.push({ name: "b.bbl", size: 20, flights: [{ duration_s: 30, kept_s: 25 }] });
        await pipeline.refreshWorkspace();
        await pipeline.scanLogs();
        disk.airframes.b_1 = {
            loop_hz: 4000,
            stick_rates: {
                rates_type: 3,
                rc_rates: [23, 23, 25],
                rates: [100, 100, 65],
                rc_expo: [62, 62, 68],
                rate_limits: [1998, 1998, 1998],
            },
            idle_rps: 120,
        };

        await pipeline.runFit();
        expect(store.activeFit).toBe("a_2+b_1");
        await pipeline.runFitEach(store.selectedFlights);
        expect(store.workspace?.fits.map((f) => f.id).sort()).toEqual(["a_2", "a_2+b_1", "b_1"]);
        expect(store.activeFit).toBe("a_2");
        expect(store.diagnostics?.flights.map((f) => f.label)).toEqual(["logs/a.bbl#2"]);

        // No FC, and a_2's log doesn't record its loop rate: nothing to compute gains for.
        await pipeline.runGains();
        expect(disk.gainsFit).toBeNull();
        expect(pipeline.gainsLoop.value).toBeNull();

        // No FC, but b_1's log does: gains for the log's loop rate, flicks at the log's max rates.
        await pipeline.selectFit("b_1");
        expect(pipeline.gainsLoop.value).toEqual({ dt: 1 / 4000, from: "log" });
        await pipeline.runGains();
        // The fit's sensor noise is extracted before its flicks are flown, for motor roughness.
        // The grid starts at the idle speed the log recorded.
        expect(disk.jobs.slice(-2)).toEqual([
            { kind: "gen", dt: null, fit: "b_1", weights: DEFAULT_WEIGHTS, grid: { resolution: 3, idleRps: 120 } },
            { kind: "noise", fit: "b_1", flight: "b.bbl#1" },
        ]);
        expect(disk.gainsFit).toBe("b_1");
        expect(store.gains).not.toBeNull();
        expect(vi.mocked(pipelineIo.flyFlicks).mock.calls.at(-1)?.[1]).toEqual({
            rps: [1000 / 360, 1000 / 360, 650 / 360],
            from: "log",
        });
        // The new gains are flown through flicks on the same fit; no matplotlib, so no plot job.
        expect(store.flicks?.fit).toBe("b_1");
        expect(disk.jobs.some((j) => j.kind === "flickPlots")).toBe(false);
        expect(pipeline.currentStep.value).toBe("upload");

        // Plots are only written on request, and their directory opened.
        await pipeline.savePlots("fit");
        expect(disk.jobs.at(-1)).toEqual({ kind: "fitPlots", fit: "b_1" });
        expect(pipelineIo.reveal).toHaveBeenLastCalledWith("/w", "fits/b_1/plots");
        await pipeline.savePlots("flicks");
        expect(disk.jobs.at(-1)).toEqual({ kind: "flickPlots" });
        expect(pipelineIo.reveal).toHaveBeenLastCalledWith("/w", "gains/plots");
        expect(store.error).toBe("");

        // A connected FC's loop rate and max rates win over the log's.
        useConnectionStore().connectionValid = true;
        store.status = { pidDt: 0.000125 } as typeof store.status;
        vi.spyOn(store, "refreshStatus").mockResolvedValue(store.status);
        // The weights and grid in the settings are what the gains are computed with, from the
        // idle speed entered in the controller settings.
        store.settings.weights = { ...DEFAULT_WEIGHTS, sigmaDeriv: 10 };
        store.settings.resolution = 4;
        store.idleRps = 130;
        await pipeline.runGains();
        // ...once.
        expect(disk.jobs.at(-1)).toEqual({
            kind: "gen",
            dt: 0.000125,
            fit: "b_1",
            weights: { ...DEFAULT_WEIGHTS, sigmaDeriv: 10 },
            grid: { resolution: 4, idleRps: 130 },
        });
        expect(store.workspace?.gainsWeights).toEqual({ ...DEFAULT_WEIGHTS, sigmaDeriv: 10 });
        store.settings.weights = { ...DEFAULT_WEIGHTS };
        store.settings.resolution = 3;
        expect(vi.mocked(pipelineIo.flyFlicks).mock.calls.at(-1)?.[1]).toMatchObject({ from: "fc" });

        await pipeline.selectFit("a_2");
        expect(pipeline.currentStep.value).toBe("gains");

        await pipeline.removeFit("a_2");
        expect(store.activeFit).toBe("b_1");
    });

    it("explores a fit's Pareto front and makes a point of it the gains", async () => {
        const { store, pipeline } = setup();
        disk.logs.push({ name: "b.bbl", size: 20, flights: [{ duration_s: 30, kept_s: 25 }] });
        disk.airframes.b_1 = { loop_hz: 4000 };
        await pipeline.refreshWorkspace();
        await pipeline.scanLogs();
        store.selectedFlights = ["b.bbl#1"];
        await pipeline.runFit();
        expect(store.activeFit).toBe("b_1");
        expect(store.front).toBeNull();

        store.settings.weights = { ...DEFAULT_WEIGHTS, tauInt: 3 };
        await pipeline.exploreFront();
        expect(store.error).toBe("");
        // The fit's noise first, for roughness; then the front at the advanced τ_int, deriving the rest.
        expect(disk.jobs.slice(-2)).toEqual([
            { kind: "noise", fit: "b_1", flight: "b.bbl#1" },
            {
                kind: "front",
                dt: null,
                fit: "b_1",
                sigmaRate: null,
                derivedSigmaRate: null,
                tauInt: 3,
                tauDeriv: null,
                derivedTauDeriv: null,
                grid: { resolution: 3, idleRps: 0 },
            },
        ]);
        expect(store.front?.fit).toBe("b_1");
        expect(store.workspace?.fits.find((f) => f.id === "b_1")?.hasFront).toBe(true);
        // Exploring computes no gains, and seeds the derived σ_rate and τ_deriv.
        expect(disk.gainsFit).toBeNull();
        expect(store.settings.weights).toEqual({ ...DEFAULT_WEIGHTS, sigmaRate: 2, tauInt: 3, tauDeriv: 0.015 });

        // At the same settings, fronts take them from the weights and keep the derived ones.
        store.settings.weights = { ...store.settings.weights, tauDeriv: 0.01 };
        await pipeline.exploreFront();
        expect(disk.jobs.at(-1)).toMatchObject({
            kind: "front",
            sigmaRate: 2,
            derivedSigmaRate: 2,
            tauDeriv: 0.01,
            derivedTauDeriv: 0.015,
        });
        expect(store.front?.tau_deriv).toBe(0.01);
        expect(store.front?.derived_tau_deriv).toBe(0.015);
        expect(store.settings.weights.tauDeriv).toBe(0.01);

        // A σ_rate set by hand is kept, with the derived one; τ_deriv is derived again at it,
        // and one set by hand is kept...
        store.settings.weights = { ...store.settings.weights, sigmaRate: 4 };
        await pipeline.exploreFront();
        expect(disk.jobs.at(-1)).toMatchObject({
            kind: "front",
            sigmaRate: 4,
            derivedSigmaRate: 2,
            tauDeriv: 0.01,
            derivedTauDeriv: null,
        });
        expect(store.front?.derived_sigma_rate).toBe(2);
        expect(store.front?.derived_tau_deriv).toBe(0.03);
        expect(store.settings.weights).toMatchObject({ sigmaRate: 4, tauDeriv: 0.01 });
        // A new τ_int derives σ_rate again too.
        store.settings.weights = { ...store.settings.weights, tauInt: 4 };
        await pipeline.exploreFront();
        expect(disk.jobs.at(-1)).toMatchObject({ kind: "front", sigmaRate: 4, derivedSigmaRate: null });
        store.settings.weights = { ...store.settings.weights, tauInt: 3 };
        await pipeline.exploreFront();
        // ...and the derived one follows.
        store.settings.weights = { ...store.settings.weights, sigmaRate: 2, tauDeriv: 0.03 };
        await pipeline.exploreFront();
        expect(disk.jobs.at(-1)).toMatchObject({ kind: "front", tauDeriv: null, derivedTauDeriv: null });
        expect(store.settings.weights.tauDeriv).toBe(0.015);
        await pipeline.exploreFront();
        expect(disk.jobs.at(-1)).toMatchObject({ kind: "front", tauDeriv: 0.015, derivedTauDeriv: 0.015 });

        await pipeline.pickFrontPoint(1);
        expect(pipelineIo.applyFrontPoint).toHaveBeenLastCalledWith("/w", "b_1", 1);
        const picked = { sigmaRate: 2, sigmaDeriv: 10, sigmaInt: 0.02, tauDeriv: 0.015, tauInt: 3 };
        expect(store.settings.weights).toEqual(picked);
        expect(store.workspace?.gainsWeights).toEqual(picked);
        expect(store.gains).not.toBeNull();
        expect(store.flicks?.fit).toBe("b_1");
        expect(pipeline.currentStep.value).toBe("upload");

        // A fit's front is read back with the fit.
        store.front = null;
        await pipeline.refreshWorkspace();
        expect((store.front as FrontReport | null)?.fit).toBe("b_1");
        store.settings.weights = { ...DEFAULT_WEIGHTS };
    });

    it("scans and selects imported logs", async () => {
        const { store, pipeline } = setup();
        await pipeline.refreshWorkspace();
        await pipeline.scanLogs();
        vi.mocked(pipelineIo.pickFiles).mockResolvedValueOnce(["/elsewhere/other.bbl"]);
        await pipeline.importLogs(false);
        expect(pipelineIo.importFiles).toHaveBeenCalledWith("/w", ["/elsewhere/other.bbl"], false);
        expect(store.selectedFlights).toEqual(["a.bbl#2", "other.bbl#1"]);

        // Cancelling the picker imports nothing.
        vi.mocked(pipelineIo.importFiles).mockClear();
        await pipeline.importLogs(true);
        expect(pipelineIo.importFiles).not.toHaveBeenCalled();
        expect(store.busy).toBeNull();
    });

    it("stops fitting each flight at the first failure", async () => {
        const { store, pipeline } = setup();
        disk.logs.push({ name: "b.bbl", size: 20, flights: [{ duration_s: 30, kept_s: 25 }] });
        await pipeline.refreshWorkspace();
        await pipeline.scanLogs();
        disk.fail.add("a_2");
        await pipeline.runFitEach(store.selectedFlights);
        expect(disk.jobs.filter((j) => j.kind === "fit")).toHaveLength(1);
        expect(disk.fits.size).toBe(0);
        expect(store.busy).toBeNull();
    });
});
