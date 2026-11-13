<template>
    <figure class="flex flex-col gap-1 min-w-0">
        <figcaption class="text-xs text-dimmed">{{ title }}</figcaption>
        <svg :viewBox="`0 0 ${WIDTH} ${HEIGHT}`" class="block w-full" role="img" :aria-label="title">
            <defs>
                <clipPath :id="clipId">
                    <rect
                        :x="MARGIN.left"
                        :y="0"
                        :width="WIDTH - MARGIN.left - MARGIN.right"
                        :height="HEIGHT - MARGIN.bottom"
                    />
                </clipPath>
            </defs>
            <rect
                v-for="(band, i) in bands"
                :key="'s' + i"
                :clip-path="`url(#${clipId})`"
                :x="band.x"
                :y="MARGIN.top"
                :width="band.width"
                :height="HEIGHT - MARGIN.top - MARGIN.bottom"
                fill="var(--chart-danger-color)"
                opacity="0.12"
            />
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
                fill="var(--chart-axis-label-color)"
            >
                {{ xLabel }}
            </text>
            <path
                v-for="(line, i) in paths"
                :key="'l' + i"
                :clip-path="`url(#${clipId})`"
                :d="line.d"
                fill="none"
                :stroke="line.color"
                :stroke-width="line.width ?? 1.2"
                :stroke-dasharray="line.dashed ? '4 3' : undefined"
            />
        </svg>
        <div class="flex flex-wrap gap-x-4 gap-y-1 text-xs">
            <span v-for="line in lines" :key="line.label" class="flex items-center gap-1">
                <span
                    class="inline-block w-3 h-0.5"
                    :style="{ background: line.color, opacity: line.dashed ? 0.6 : 1 }"
                ></span>
                {{ line.label }}
            </span>
            <span v-if="bands.length" class="flex items-center gap-1">
                <span class="inline-block size-2 opacity-30" style="background: var(--chart-danger-color)"></span>
                {{ $t("whippyflightFlickSaturated") }}
            </span>
        </div>
    </figure>
</template>

<script setup lang="ts">
import { computed, useId } from "vue";
import * as d3 from "d3";

export interface FlickLine {
    values: number[];
    color: string;
    label: string;
    dashed?: boolean;
    width?: number;
}

/** Stick-flick traces as lines over time, saturated stretches shaded. */
const props = defineProps<{
    title: string;
    xLabel: string;
    /** Sample spacing, s; the x axis is in ms. */
    dt: number;
    lines: FlickLine[];
    /** The ms span shown, if not all of it. */
    span?: [number, number];
    /** Per sample, the fraction of it the motors saturated. */
    saturated?: number[];
}>();

const WIDTH = 640;
const HEIGHT = 240;
const MARGIN = { top: 8, right: 8, bottom: 30, left: 44 };

const clipId = `flick-clip-${useId()}`;
const length = computed(() => Math.max(0, ...props.lines.map((l) => l.values.length)));
const dtMs = computed(() => 1e3 * props.dt);
const span = computed<[number, number]>(() => props.span ?? [0, Math.max(dtMs.value, (length.value - 1) * dtMs.value)]);

const xScale = computed(() =>
    d3
        .scaleLinear()
        .domain(span.value)
        .range([MARGIN.left, WIDTH - MARGIN.right]),
);
/** Over the whole flick, not just the span, so a flick's zooms share one y axis. */
const yScale = computed(() => {
    const [lo, hi] = d3.extent(props.lines.flatMap((l) => l.values)) as [number | undefined, number | undefined];
    const domain: [number, number] = lo === undefined || hi === undefined || lo === hi ? [0, 1] : [lo, hi];
    return d3
        .scaleLinear()
        .domain(domain)
        .range([HEIGHT - MARGIN.bottom, MARGIN.top])
        .nice();
});
const xTicks = computed(() => xScale.value.ticks(6));
const yTicks = computed(() => yScale.value.ticks(5));
const format = d3.format("~g");

const paths = computed(() =>
    props.lines.map((line) => ({
        ...line,
        d:
            d3
                .line<number>()
                .x((_, i) => xScale.value(i * dtMs.value))
                .y((v) => yScale.value(v))(line.values) ?? "",
    })),
);

/** Runs of saturated samples, as x extents. */
const bands = computed(() => {
    const out: { x: number; width: number }[] = [];
    const sat = props.saturated ?? [];
    let start = -1;
    for (let i = 0; i <= sat.length; i++) {
        const on = i < sat.length && sat[i] > 0;
        if (on && start < 0) {
            start = i;
        } else if (!on && start >= 0) {
            const x = xScale.value(start * dtMs.value);
            out.push({ x, width: Math.max(1, xScale.value(i * dtMs.value) - x) });
            start = -1;
        }
    }
    return out;
});
</script>
