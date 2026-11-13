<template>
    <div class="flex flex-col gap-2">
        <p v-if="stale" class="text-sm text-warning">{{ stale }}</p>
        <p v-if="!report.gyro_filters" class="text-sm text-warning">{{ $t("whippyflightFrontNoFilters") }}</p>
        <div class="grid gap-3 grid-cols-1 md:grid-cols-2">
            <FrontScatter
                v-for="chart in charts"
                :key="chart.title"
                :title="chart.title"
                :x-label="chart.xLabel"
                :y-label="chart.yLabel"
                :better="chart.better"
                :points="chart.points"
                :lines="chart.lines"
                :hovered="hovered"
                :current="current"
                @hover="hovered = $event"
                @pick="!disabled && emit('pick', $event)"
            />
        </div>
        <p class="text-sm tabular-nums min-h-5">
            <template v-if="shown">
                <span class="font-mono text-dimmed">
                    σ_deriv {{ shown.sigma_deriv }} · σ_int {{ shown.sigma_int }}
                </span>
                — {{ describe(shown) }}
                <span v-if="hovered !== null && hovered !== current" class="text-dimmed">
                    · {{ $t("whippyflightFrontClick") }}
                </span>
            </template>
        </p>
        <p class="text-xs text-dimmed">
            {{ $t("whippyflightFrontNote", { n: report.points.length, low: LOW_MARGIN_MS }) }}
        </p>
    </div>
</template>

<script setup lang="ts">
import { computed, ref } from "vue";
import { useTranslation } from "i18next-vue";
import FrontScatter, { type ScatterPoint } from "./FrontScatter.vue";
import { paretoOptimal, type FrontMeasure, type FrontPoint, type FrontReport } from "@/js/whippyflight/front";

const props = defineProps<{
    report: FrontReport;
    /** The current gains' point; −1 if they aren't on the front. */
    current: number;
    /** Why the front no longer matches what gains would be computed for, if it doesn't. */
    stale?: string;
    disabled?: boolean;
}>();
const emit = defineEmits<{ pick: [index: number] }>();

const { t } = useTranslation();
const hovered = ref<number | null>(null);
/** Delay margin, ms, below which a point is drawn as a warning. */
const LOW_MARGIN_MS = 1.5;

const shown = computed(() => props.report.points[hovered.value ?? props.current] as FrontPoint | undefined);
const hasRoughness = computed(() => props.report.points.every((p) => p.rough_v !== null));

const value = (p: FrontPoint, m: FrontMeasure): number => (m === "rough_v" ? 1e3 * (p.rough_v ?? 0) : p[m]);

const labels = computed<Record<FrontMeasure, string>>(() => ({
    kick_iae_ms: t("whippyflightFrontKick"),
    rough_v: t("whippyflightFrontRough"),
    delay_margin_ms: t("whippyflightFrontMargin"),
    trim_deg: t("whippyflightFrontTrim"),
}));
const UNITS: Record<FrontMeasure, string> = { kick_iae_ms: "ms", rough_v: "mV", delay_margin_ms: "ms", trim_deg: "°" };

/** Runs of points with the same `key`, in order of `along`. */
function runs(key: "sigma_deriv" | "sigma_int", along: "sigma_deriv" | "sigma_int"): number[][] {
    const groups = new Map<number, number[]>();
    props.report.points.forEach((p, i) => groups.set(p[key], [...(groups.get(p[key]) ?? []), i]));
    return [...groups.values()].map((run) =>
        run.sort((a, b) => props.report.points[a][along] - props.report.points[b][along]),
    );
}

const charts = computed(() => {
    const chart = (
        title: string,
        x: FrontMeasure,
        y: FrontMeasure,
        better: string,
        lines: number[][],
    ): {
        title: string;
        xLabel: string;
        yLabel: string;
        better: string;
        points: ScatterPoint[];
        lines: number[][];
    } => {
        const optimal = paretoOptimal(props.report.points, [x, y]);
        return {
            title,
            xLabel: `${labels.value[x]}, ${UNITS[x]}`,
            yLabel: `${labels.value[y]}, ${UNITS[y]}`,
            better,
            lines,
            points: props.report.points.map((p, index) => ({
                index,
                x: value(p, x),
                y: value(p, y),
                optimal: optimal[index],
                warn: p.delay_margin_ms < LOW_MARGIN_MS,
            })),
        };
    };
    return [
        hasRoughness.value
            ? chart(
                  t("whippyflightFrontResponse"),
                  "kick_iae_ms",
                  "rough_v",
                  t("whippyflightFrontBetterLowerLeft"),
                  runs("sigma_int", "sigma_deriv"),
              )
            : chart(
                  t("whippyflightFrontResponse"),
                  "kick_iae_ms",
                  "delay_margin_ms",
                  t("whippyflightFrontBetterUpperLeft"),
                  runs("sigma_int", "sigma_deriv"),
              ),
        chart(
            t("whippyflightFrontHold"),
            "trim_deg",
            "delay_margin_ms",
            t("whippyflightFrontBetterUpperLeft"),
            runs("sigma_deriv", "sigma_int"),
        ),
    ];
});

function describe(p: FrontPoint): string {
    const margin =
        p.delay_margin_ms < 0
            ? t("whippyflightFlickMarginNone")
            : `${p.delay_margin_ms >= props.report.max_margin_ms ? "≥ " : ""}${p.delay_margin_ms.toFixed(1)} ms`;
    return [
        `${labels.value.kick_iae_ms} ${p.kick_iae_ms.toFixed(0)} ms`,
        p.rough_v === null ? null : `${labels.value.rough_v} ${(1e3 * p.rough_v).toFixed(1)} mV`,
        `${labels.value.delay_margin_ms} ${margin}`,
        `${labels.value.trim_deg} ${p.trim_deg.toFixed(1)}°`,
    ]
        .filter((s) => s !== null)
        .join(" · ");
}
</script>
