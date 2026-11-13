import { describe, expect, it } from "vitest";
import {
    dominates,
    frontIndexOf,
    paretoOptimal,
    type FrontPoint,
    type FrontReport,
} from "../../src/js/whippyflight/front";

const point = (over: Partial<FrontPoint>): FrontPoint => ({
    sigma_deriv: 15,
    sigma_int: 0.012,
    kick_iae_ms: 50,
    rough_v: 0.003,
    delay_margin_ms: 3,
    trim_deg: 6,
    ...over,
});

describe("WhippyFlight Pareto front", () => {
    it("dominates only when no worse anywhere and better somewhere", () => {
        const base = point({});
        expect(dominates(point({ kick_iae_ms: 40 }), base)).toBe(true);
        // More delay margin is better; the rest lower.
        expect(dominates(point({ delay_margin_ms: 4 }), base)).toBe(true);
        expect(dominates(point({ delay_margin_ms: 2 }), base)).toBe(false);
        expect(dominates(point({ kick_iae_ms: 40, rough_v: 0.004 }), base)).toBe(false);
        expect(dominates(base, point({}))).toBe(false);
    });

    it("compares without roughness when a point lacks it", () => {
        expect(dominates(point({ rough_v: null, trim_deg: 5 }), point({}))).toBe(true);
    });

    it("marks the points nothing dominates", () => {
        const points = [
            point({ kick_iae_ms: 40, rough_v: 0.005 }),
            point({ kick_iae_ms: 60, rough_v: 0.002 }),
            point({ kick_iae_ms: 70, rough_v: 0.006 }),
        ];
        expect(paretoOptimal(points)).toEqual([true, true, false]);
        // On kick recovery and delay margin alone (equal margins), only the fastest.
        expect(paretoOptimal(points, ["kick_iae_ms", "delay_margin_ms"])).toEqual([true, false, false]);
    });

    it("finds the point with given weights", () => {
        const report = {
            sigma_rate: 5,
            points: [point({}), point({ sigma_deriv: 10 })],
        } as FrontReport;
        expect(frontIndexOf(report, { sigmaRate: 5, sigmaDeriv: 10, sigmaInt: 0.012 })).toBe(1);
        expect(frontIndexOf(report, { sigmaRate: 2, sigmaDeriv: 10, sigmaInt: 0.012 })).toBe(-1);
        expect(frontIndexOf(report, null)).toBe(-1);
    });
});
