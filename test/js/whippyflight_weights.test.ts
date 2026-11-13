import { describe, expect, it } from "vitest";
import {
    DEFAULT_WEIGHTS,
    INTEGRAL_RANGE,
    sameWeights,
    SLIDER_MAX,
    sliderOf,
    STIFFNESS_RANGE,
    weightAt,
} from "../../src/js/whippyflight/weights";

describe("WhippyFlight weight sliders", () => {
    it("span their ranges on a log scale", () => {
        expect(weightAt(0, STIFFNESS_RANGE)).toBe(5);
        expect(weightAt(SLIDER_MAX, STIFFNESS_RANGE)).toBe(30);
        expect(weightAt(SLIDER_MAX / 2, INTEGRAL_RANGE)).toBe(0.0141);
        expect(sliderOf(Math.sqrt(5 * 30), STIFFNESS_RANGE)).toBeCloseTo(SLIDER_MAX / 2);
    });

    it("put the defaults inside their ranges", () => {
        for (const [value, range] of [
            [DEFAULT_WEIGHTS.sigmaDeriv, STIFFNESS_RANGE],
            [DEFAULT_WEIGHTS.sigmaInt, INTEGRAL_RANGE],
        ] as const) {
            const position = sliderOf(value, range);
            expect(position).toBeGreaterThan(0);
            expect(position).toBeLessThan(SLIDER_MAX);
            expect(weightAt(position, range)).toBe(value);
        }
    });

    it("clamp values set outside a range to its ends", () => {
        expect(sliderOf(1, STIFFNESS_RANGE)).toBe(0);
        expect(sliderOf(100, STIFFNESS_RANGE)).toBe(SLIDER_MAX);
    });

    it("compare by value", () => {
        expect(sameWeights({ ...DEFAULT_WEIGHTS }, DEFAULT_WEIGHTS)).toBe(true);
        expect(sameWeights({ ...DEFAULT_WEIGHTS, sigmaInt: 0.02 }, DEFAULT_WEIGHTS)).toBe(false);
    });
});
