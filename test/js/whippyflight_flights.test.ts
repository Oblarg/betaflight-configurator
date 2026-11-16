import { beforeEach, describe, expect, it } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { fitId, flightSpec, listFlights, specOfLabel, unscannedLogs } from "../../src/js/whippyflight/flights";
import {
    DEFAULT_FLICK_CONDITIONS,
    DEFAULT_FLICK_FEEDBACK,
    controllerLagMs,
    flownAtRates,
    sameConditions,
    sameFeedback,
    validFlickConditions,
    validFlickFeedback,
    logMaxRatesRps,
    maxRatesRps,
    type FlickMetrics,
    type FlickReport,
} from "../../src/js/whippyflight/flicks";
import RateCurve from "../../src/js/RateCurve";
import { RATES_TYPE, useFlightControllerStore } from "../../src/stores/fc";
import type { FlightIndex, LogFile } from "../../src/js/whippyflight/pipeline";

const logs: LogFile[] = [
    { name: "mako_20241103_183300.bbl", size: 100 },
    { name: "mako_20241104_100000.bbl", size: 200 },
];

const index: FlightIndex = {
    "mako_20241103_183300.bbl": {
        size: 100,
        flights: [
            { index: 1, duration_s: 0.6, reason: "shorter than 10 s" },
            { index: 2, duration_s: 163, kept_s: 153, reason: null },
            { index: 3, duration_s: 40, kept_s: 2, reason: "only 2.0 s airborne and impact-free" },
        ],
    },
    "mako_20241104_100000.bbl": { size: 150, flights: [{ index: 1, duration_s: 90, kept_s: 80, reason: null }] },
};

describe("WhippyFlight flight selection", () => {
    it("lists flights only for logs scanned at their current size", () => {
        const flights = listFlights(index, logs);
        expect(flights.map((f) => [f.spec, f.usable])).toEqual([
            ["mako_20241103_183300.bbl#1", false],
            ["mako_20241103_183300.bbl#2", true],
            ["mako_20241103_183300.bbl#3", false],
        ]);
        expect(unscannedLogs(index, logs)).toEqual(["mako_20241104_100000.bbl"]);
        expect(unscannedLogs(null, logs)).toEqual(logs.map((l) => l.name));
        expect(listFlights(null, logs)).toEqual([]);
    });

    it("names a fit by its flights, independent of order", () => {
        const a = flightSpec("mako_1.bbl", 3);
        const b = flightSpec("mako_2.BFL", 1);
        expect(fitId([b, a], "auto")).toBe("mako_1_3+mako_2_1");
        expect(fitId([a, b], "auto")).toBe(fitId([b, a], "auto"));
        expect(fitId([a], "off")).toBe("mako_1_3_drag-off");
        expect(fitId(["weird name!.bbl#2"], "auto")).toBe("weird_name__2");
    });

    it("abbreviates long fit names but keeps them distinct", () => {
        const many = Array.from({ length: 12 }, (_, i) => flightSpec(`craft_2024110${i % 10}_120000.bbl`, i + 1));
        const id = fitId(many, "auto");
        expect(id.length).toBeLessThan(80);
        expect(id).toMatch(/^craft_20241100_120000_1\+11more_[0-9a-f]{8}$/);
        expect(fitId(many.slice(1), "auto")).not.toBe(id);
        expect(id).toMatch(/^[A-Za-z0-9_+-]+$/);
    });

    it("maps diagnostics labels back to specs", () => {
        expect(specOfLabel("logs/mako_1.bbl#3")).toBe("mako_1.bbl#3");
    });
});

describe("WhippyFlight flicks", () => {
    beforeEach(() => setActivePinia(createPinia()));

    it("flies full stick at the FC's max rates", () => {
        const fc = useFlightControllerStore();
        Object.assign(fc.rcTuning, {
            rates_type: RATES_TYPE.ACTUAL,
            RC_RATE: 0.07,
            rcPitchRate: 0.07,
            roll_rate: 0.67,
            pitch_rate: 0.5,
            rcYawRate: 0.07,
            yaw_rate: 0.4,
            RC_EXPO: 0,
            RC_PITCH_EXPO: 0,
            RC_YAW_EXPO: 0,
        });
        const [roll, pitch, yaw] = maxRatesRps(new RateCurve(false));
        expect(roll * 360).toBeCloseTo(670, 0);
        expect(pitch * 360).toBeCloseTo(500, 0);
        expect(yaw * 360).toBeCloseTo(400, 0);
    });

    it("reads max rates from a blackbox header, whatever the FC's", () => {
        // Headers hold rates ×100; a Meteor65 log on Actual rates.
        const [roll, pitch, yaw] = logMaxRatesRps({
            rates_type: RATES_TYPE.ACTUAL,
            rc_rates: [23, 23, 25],
            rates: [100, 100, 65],
            rc_expo: [62, 62, 68],
            rate_limits: [1998, 1998, 1998],
        });
        expect(roll * 360).toBeCloseTo(1000, 0);
        expect(pitch * 360).toBeCloseTo(1000, 0);
        expect(yaw * 360).toBeCloseTo(650, 0);
    });

    it("counts the controller's lag behind the target, not the smoother's", () => {
        expect(controllerLagMs({ gyro_lag_ms: 28.5, target_lag_ms: 26.25 } as FlickMetrics)).toBeCloseTo(2.25);
    });
});

describe("flownAtRates", () => {
    const report = (from: FlickReport["rates_from"], rps: number[]) =>
        ({
            rates_from: from,
            flicks: ["roll", "pitch", "yaw"].flatMap((axis, i) => [
                { axis, amp: rps[i] },
                { axis, amp: -rps[i] },
            ]),
        }) as unknown as FlickReport;

    it("matches hand-set rates only when flown at them", () => {
        expect(flownAtRates(report("manual", [2.2, 2.2, 1.4]), [2.2, 2.2, 1.4])).toBe(true);
        expect(flownAtRates(report("manual", [2.2, 2.2, 1.4]), [2.2, 2.2, 1.1])).toBe(false);
        expect(flownAtRates(report("fc", [2.2, 2.2, 1.4]), [2.2, 2.2, 1.4])).toBe(false);
    });

    it("matches profile rates when none are set by hand", () => {
        expect(flownAtRates(report("log", [2.8, 2.8, 1.8]), null)).toBe(true);
        expect(flownAtRates(report("manual", [2.8, 2.8, 1.8]), null)).toBe(false);
    });
});

describe("validFlickConditions", () => {
    it("fills what isn't stored, or isn't valid, with the defaults", () => {
        expect(validFlickConditions(undefined)).toEqual(DEFAULT_FLICK_CONDITIONS);
        const stored = validFlickConditions({
            gyro_filter: false,
            motor_noise: false,
            gyro_noise_rms: 2,
            motor_speed_noise_rms: -1,
            cg_shift_pct: [5, "x"],
            turbulence: true,
            turbulence_rms: [300, 200, 100],
            turbulence_hz: 0,
            actuator_delay_ms: 25,
            telemetry_delay_ms: 3.2,
        });
        expect(stored).toEqual({
            ...DEFAULT_FLICK_CONDITIONS,
            gyro_filter: false,
            motor_noise: false,
            gyro_noise_rms: 2,
            cg_shift_pct: [5, 0],
            turbulence: true,
            turbulence_rms: [300, 200, 100],
            telemetry_delay_ms: 3.2,
        });
        expect(validFlickConditions({ turbulence_rms: [300, -1, 100] }).turbulence_rms).toBeNull();
        expect(validFlickConditions({ turbulence_rms: 300 }).turbulence_rms).toBeNull();
        expect(sameConditions(stored, DEFAULT_FLICK_CONDITIONS)).toBe(false);
        expect(sameConditions(validFlickConditions({}), DEFAULT_FLICK_CONDITIONS)).toBe(true);
    });
});

describe("validFlickFeedback", () => {
    it("keeps every gain unless one is stored off, and carries over feedforward-only", () => {
        expect(validFlickFeedback(undefined)).toEqual(DEFAULT_FLICK_FEEDBACK);
        expect(validFlickFeedback({ i: false, d: "x" })).toEqual({ p: true, i: false, d: true });
        expect(validFlickFeedback(undefined, true)).toEqual({ p: false, i: false, d: false });
        expect(validFlickFeedback({ p: true, i: true, d: false }, true)).toEqual({ p: true, i: true, d: false });
        expect(sameFeedback(validFlickFeedback({ d: false }), DEFAULT_FLICK_FEEDBACK)).toBe(false);
        expect(sameFeedback(validFlickFeedback({}), DEFAULT_FLICK_FEEDBACK)).toBe(true);
    });
});
