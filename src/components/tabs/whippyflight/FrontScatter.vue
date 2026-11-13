<template>
    <figure class="flex flex-col gap-1 min-w-0">
        <figcaption class="text-xs text-dimmed flex justify-between">
            <span>{{ title }}</span>
            <span>{{ better }}</span>
        </figcaption>
        <svg
            :viewBox="`0 0 ${WIDTH} ${HEIGHT}`"
            class="block w-full"
            role="img"
            :aria-label="title"
            @mouseleave="emit('hover', null)"
        >
            <g v-for="tick in yTicks" :key="'y' + tick">
                <line
                    :x1="MARGIN.left"
                    :x2="WIDTH - MARGIN.right"
                    :y1="yScale(tick)"
                    :y2="yScale(tick)"
                    stroke="var(--chart-grid-line-color)"
                    stroke-width="0.5"
                />
                <text
                    :x="MARGIN.left - 4"
                    :y="yScale(tick)"
                    text-anchor="end"
                    dominant-baseline="middle"
                    font-size="10"
                    fill="var(--chart-tick-color)"
                >
                    {{ format(tick) }}
                </text>
            </g>
            <text
                v-for="tick in xTicks"
                :key="'x' + tick"
                :x="xScale(tick)"
                :y="HEIGHT - MARGIN.bottom + 12"
                text-anchor="middle"
                font-size="10"
                fill="var(--chart-tick-color)"
            >
                {{ format(tick) }}
            </text>
            <text
                :x="(MARGIN.left + WIDTH - MARGIN.right) / 2"
                :y="HEIGHT - 2"
                text-anchor="middle"
                font-size="10"
                fill="var(--chart-tick-color)"
            >
                {{ xLabel }}
            </text>
            <text
                :transform="`translate(10 ${(MARGIN.top + HEIGHT - MARGIN.bottom) / 2}) rotate(-90)`"
                text-anchor="middle"
                font-size="10"
                fill="var(--chart-tick-color)"
            >
                {{ yLabel }}
            </text>
            <path
                v-for="(line, i) in paths"
                :key="'l' + i"
                :d="line"
                fill="none"
                stroke="var(--chart-grid-line-color)"
                stroke-width="1"
            />
            <g
                v-for="p in points"
                :key="p.index"
                class="cursor-pointer"
                @mouseenter="emit('hover', p.index)"
                @click="emit('pick', p.index)"
            >
                <circle :cx="xScale(p.x)" :cy="yScale(p.y)" r="9" fill="transparent" />
                <circle
                    v-if="p.index === current"
                    :cx="xScale(p.x)"
                    :cy="yScale(p.y)"
                    r="8"
                    fill="none"
                    stroke="var(--chart-reference-color)"
                    stroke-width="1.5"
                />
                <circle
                    :cx="xScale(p.x)"
                    :cy="yScale(p.y)"
                    :r="p.index === hovered ? 6 : 4"
                    :fill="p.optimal ? color(p) : 'none'"
                    :stroke="color(p)"
                    stroke-width="1.2"
                />
            </g>
        </svg>
    </figure>
</template>

<script setup lang="ts">
import { computed } from "vue";
import * as d3 from "d3";

export interface ScatterPoint {
    /** Index into the front, shared by every chart of it. */
    index: number;
    x: number;
    y: number;
    /** Not dominated by any other point. */
    optimal: boolean;
    /** Drawn in the warning color. */
    warn: boolean;
}

const props = defineProps<{
    title: string;
    xLabel: string;
    yLabel: string;
    /** Which way is better, e.g. "better ↙". */
    better: string;
    points: ScatterPoint[];
    /** Runs of point indices to join with faint lines. */
    lines: number[][];
    hovered: number | null;
    /** The current gains' point, ringed. */
    current: number;
}>();
const emit = defineEmits<{ hover: [index: number | null]; pick: [index: number] }>();

const WIDTH = 480;
const HEIGHT = 260;
const MARGIN = { top: 10, right: 12, bottom: 30, left: 44 };
const POINT_COLOR = "var(--color-axis-pitch)";
const WARN_COLOR = "var(--chart-danger-color)";

function padded(values: number[]): [number, number] {
    const [lo, hi] = d3.extent(values) as [number, number];
    const pad = (hi - lo) * 0.06 || Math.abs(hi) * 0.1 || 1;
    return [lo - pad, hi + pad];
}

const xScale = computed(() =>
    d3
        .scaleLinear()
        .domain(padded(props.points.map((p) => p.x)))
        .range([MARGIN.left, WIDTH - MARGIN.right]),
);
const yScale = computed(() =>
    d3
        .scaleLinear()
        .domain(padded(props.points.map((p) => p.y)))
        .range([HEIGHT - MARGIN.bottom, MARGIN.top]),
);
const xTicks = computed(() => xScale.value.ticks(6));
const yTicks = computed(() => yScale.value.ticks(5));

const paths = computed(() => {
    const byIndex = new Map(props.points.map((p) => [p.index, p]));
    const line = d3
        .line<ScatterPoint>()
        .x((p) => xScale.value(p.x))
        .y((p) => yScale.value(p.y));
    return props.lines.map((run) => line(run.flatMap((i) => byIndex.get(i) ?? [])) ?? "");
});

function color(p: ScatterPoint): string {
    return p.warn ? WARN_COLOR : POINT_COLOR;
}

function format(value: number): string {
    return String(Number(value.toPrecision(3)));
}
</script>
