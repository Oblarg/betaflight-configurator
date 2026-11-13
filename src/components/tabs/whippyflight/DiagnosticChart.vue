<template>
    <figure class="flex flex-col gap-1 min-w-0">
        <figcaption class="text-xs text-dimmed">{{ title }}</figcaption>
        <svg :viewBox="`0 0 ${WIDTH} ${HEIGHT}`" class="block w-full" role="img" :aria-label="title">
            <g v-for="tick in yTicks" :key="'y' + tick">
                <line
                    :x1="MARGIN.left"
                    :x2="WIDTH - MARGIN.right"
                    :y1="y(tick)"
                    :y2="y(tick)"
                    stroke="var(--chart-grid-line-color)"
                    stroke-width="0.5"
                />
                <text
                    :x="MARGIN.left - 4"
                    :y="y(tick)"
                    text-anchor="end"
                    dominant-baseline="middle"
                    font-size="10"
                    fill="var(--chart-tick-color)"
                >
                    {{ format(tick) }}
                </text>
            </g>
            <g v-for="tick in xTicks" :key="'x' + tick">
                <text
                    :x="x(tick)"
                    :y="HEIGHT - MARGIN.bottom + 12"
                    text-anchor="middle"
                    font-size="10"
                    fill="var(--chart-tick-color)"
                >
                    {{ format(tick) }}
                </text>
            </g>
            <text
                :x="(MARGIN.left + WIDTH - MARGIN.right) / 2"
                :y="HEIGHT - 2"
                text-anchor="middle"
                font-size="10"
                fill="var(--chart-axis-label-color)"
            >
                {{ xLabel }}
            </text>

            <template v-if="kind === 'histogram'">
                <rect
                    v-for="(bar, i) in bars"
                    :key="i"
                    :x="bar.x"
                    :y="bar.y"
                    :width="bar.width"
                    :height="bar.height"
                    fill="var(--color-axis-yaw)"
                    opacity="0.8"
                />
            </template>
            <template v-else>
                <line
                    v-if="kind === 'scatter'"
                    :x1="x(xDomain[0])"
                    :y1="y(xDomain[0])"
                    :x2="x(xDomain[1])"
                    :y2="y(xDomain[1])"
                    stroke="var(--chart-reference-color)"
                    stroke-dasharray="4 3"
                />
                <path :d="dots(seriesA)" :fill="kind === 'scatter' ? 'var(--color-axis-yaw)' : MEASURED_COLOR" />
                <path v-if="kind === 'series'" :d="dots(seriesB)" :fill="PREDICTED_COLOR" />
            </template>
        </svg>
        <div v-if="kind === 'series'" class="flex gap-4 text-xs">
            <span class="flex items-center gap-1">
                <span class="inline-block size-2 rounded-full" :style="{ background: MEASURED_COLOR }"></span>
                {{ $t("whippyflightMeasured") }}
            </span>
            <span class="flex items-center gap-1">
                <span class="inline-block size-2 rounded-full" :style="{ background: PREDICTED_COLOR }"></span>
                {{ $t("whippyflightPredicted") }}
            </span>
        </div>
    </figure>
</template>

<script setup lang="ts">
import { computed } from "vue";
import * as d3 from "d3";

/**
 * One fit-diagnostics chart: a measured/predicted time series, predicted against measured,
 * or the residual histogram. Points are drawn as one path of tiny squares, which stays cheap
 * at the 1500 points fit_diagnostics.py samples.
 */
const props = defineProps<{
    kind: "series" | "scatter" | "histogram";
    title: string;
    xLabel: string;
    /** series: time; scatter: measured; histogram: bin edges. */
    x: number[];
    /** series: measured; scatter: predicted; histogram: counts. */
    a: number[];
    /** series: predicted. */
    b?: number[];
    /** series: the x extent to show, if not the data's. */
    xSpan?: [number, number];
}>();

const WIDTH = 420;
const HEIGHT = 240;
const MARGIN = { top: 8, right: 8, bottom: 30, left: 44 };
const MEASURED_COLOR = "var(--color-axis-pitch)";
const PREDICTED_COLOR = "var(--color-axis-roll)";
const DOT = 1.6;

function extent(values: number[]): [number, number] {
    const [lo, hi] = d3.extent(values) as [number | undefined, number | undefined];
    if (lo === undefined || hi === undefined) {
        return [0, 1];
    }
    return lo === hi ? [lo - 1, hi + 1] : [lo, hi];
}

const xDomain = computed<[number, number]>(() =>
    props.kind === "scatter" ? extent([...props.x, ...props.a]) : (props.xSpan ?? extent(props.x)),
);
const yDomain = computed<[number, number]>(() => {
    if (props.kind === "histogram") {
        return [0, Math.max(1, ...props.a)];
    }
    if (props.kind === "scatter") {
        return xDomain.value;
    }
    return extent([...props.a, ...(props.b ?? [])]);
});

const xScale = computed(() =>
    d3
        .scaleLinear()
        .domain(xDomain.value)
        .range([MARGIN.left, WIDTH - MARGIN.right])
        .nice(),
);
const yScale = computed(() =>
    d3
        .scaleLinear()
        .domain(yDomain.value)
        .range([HEIGHT - MARGIN.bottom, MARGIN.top])
        .nice(),
);
const x = (v: number) => xScale.value(v);
const y = (v: number) => yScale.value(v);
const xTicks = computed(() => xScale.value.ticks(5));
const yTicks = computed(() => yScale.value.ticks(5));
const format = d3.format("~g");

const seriesA = computed(() => zip(props.x, props.a));
const seriesB = computed(() => zip(props.x, props.b ?? []));

function zip(xs: number[], ys: number[]): [number, number][] {
    const n = Math.min(xs.length, ys.length);
    return Array.from({ length: n }, (_, i) => [xs[i], ys[i]]);
}

function dots(points: [number, number][]): string {
    return points
        .map(([px, py]) => `M${(x(px) - DOT / 2).toFixed(1)},${(y(py) - DOT / 2).toFixed(1)}h${DOT}v${DOT}h-${DOT}z`)
        .join("");
}

const bars = computed(() =>
    props.a.map((count, i) => {
        const left = x(props.x[i]);
        const right = x(props.x[i + 1] ?? props.x[i]);
        return {
            x: left,
            width: Math.max(0, right - left - 0.5),
            y: y(count),
            height: y(0) - y(count),
        };
    }),
);
</script>
