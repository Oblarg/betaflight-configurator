<template>
    <div class="flex flex-col gap-2">
        <p v-if="stale" class="flex flex-wrap items-center gap-x-2 text-sm text-warning">
            {{ stale }}
            <UButton
                size="xs"
                variant="link"
                icon="i-lucide-refresh-cw"
                :label="$t('whippyflightFrontRecompute')"
                :disabled="recomputeDisabled"
                @click="emit('recompute')"
            />
        </p>
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
                @pick="pickable && emit('pick', $event)"
            />
        </div>
        <p class="text-sm tabular-nums min-h-5">
            <template v-if="shown">
                <span class="font-mono text-dimmed">
                    σ_deriv {{ shown.sigma_deriv }} · σ_int {{ shown.sigma_int }}
                </span>
                — {{ describe(shown) }}
                <span v-if="pickable && hovered !== null && hovered !== current" class="text-dimmed">
                    · {{ $t("whippyflightFrontClick") }}
                </span>
            </template>
        </p>
        <p class="text-xs text-dimmed">
            {{
                $t("whippyflightFrontNote", {
                    n: flyable.length,
                    hidden: report.points.length - flyable.length,
                    tauDeriv: Number((1e3 * report.tau_deriv).toPrecision(3)),
                    tauInt: Number(report.tau_int.toPrecision(3)),
                })
            }}
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
    /** Why the front no longer matches what gains would be computed for, if it doesn't; it can't be picked from then. */
    stale?: string;
    disabled?: boolean;
    recomputeDisabled?: boolean;
}>();
const emit = defineEmits<{ pick: [index: number]; recompute: [] }>();

const { t } = useTranslation();
const hovered = ref<number | null>(null);
const pickable = computed(() => !props.disabled && !props.stale);
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

/** Points the sim flies cleanly; the rest aren't drawn. */
const flyable = computed(() => props.report.points.flatMap((p, index) => (p.delay_margin_ms >= 0 ? [index] : [])));

/** Runs of flyable points on the same grid level of weight `key` (0: σ_deriv, 1: σ_int), along the other. */
function runs(key: 0 | 1): number[][] {
    const groups = new Map<number, number[]>();
    flyable.value.forEach((i) => {
        const level = props.report.points[i].grid[key];
        groups.set(level, [...(groups.get(level) ?? []), i]);
    });
    const along = 1 - key;
    return [...groups.values()].map((run) =>
        run.sort((a, b) => props.report.points[a].grid[along] - props.report.points[b].grid[along]),
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
        const points = flyable.value.map((index) => props.report.points[index]);
        const optimal = paretoOptimal(points, [x, y]);
        return {
            title,
            xLabel: `${labels.value[x]}, ${UNITS[x]}`,
            yLabel: `${labels.value[y]}, ${UNITS[y]}`,
            better,
            lines,
            points: points.map((p, k) => ({
                index: flyable.value[k],
                x: value(p, x),
                y: value(p, y),
                optimal: optimal[k],
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
                  runs(1),
              )
            : chart(
                  t("whippyflightFrontResponse"),
                  "kick_iae_ms",
                  "delay_margin_ms",
                  t("whippyflightFrontBetterUpperLeft"),
                  runs(1),
              ),
        chart(
            t("whippyflightFrontHold"),
            "trim_deg",
            "delay_margin_ms",
            t("whippyflightFrontBetterUpperLeft"),
            runs(0),
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
