<template>
    <div class="flex flex-col gap-3">
        <p v-if="report.rates_from === 'default'" class="text-sm text-warning">
            {{ $t("whippyflightFlickDefaultRates") }}
        </p>

        <div class="flex flex-wrap gap-x-6 gap-y-1 text-sm">
            <span
                class="flex gap-1"
                :title="$t('whippyflightFlickMarginHint', { max: report.max_margin_ms.toFixed(0) })"
            >
                <span class="text-dimmed">{{ $t("whippyflightFlickMargin") }}</span>
                <span class="tabular-nums" :class="marginLow ? 'text-warning' : ''">{{ margin }}</span>
            </span>
            <span class="flex gap-1" :title="$t('whippyflightFlickTrimHint')">
                <span class="text-dimmed">{{ $t("whippyflightFlickTrim") }}</span>
                <span class="tabular-nums">{{
                    report.trim_deg === null ? "–" : `${report.trim_deg.toFixed(1)}°`
                }}</span>
            </span>
            <span class="flex gap-1" :title="$t('whippyflightFlickStickTrackingHint')">
                <span class="text-dimmed">{{ $t("whippyflightFlickStickTracking") }}</span>
                <span class="tabular-nums">{{ report.stick_tracking }}</span>
                <span class="text-dimmed">{{
                    $t(
                        report.stick_tracking_from === "fc"
                            ? "whippyflightFlickStickTrackingFc"
                            : "whippyflightFlickStickTrackingLog",
                    )
                }}</span>
            </span>
            <span
                v-if="report.hover_rough_v !== null"
                class="flex gap-1"
                :title="$t('whippyflightFlickHoverRoughHint')"
            >
                <span class="text-dimmed">{{ $t("whippyflightFlickHoverRough") }}</span>
                <span class="tabular-nums">{{ millivolts(report.hover_rough_v, 1) }}</span>
                <span v-if="report.bf_rough_v !== null" class="text-dimmed">
                    {{ $t("whippyflightFlickBfRough", { bf: millivolts(report.bf_rough_v) }) }}
                </span>
            </span>
        </div>

        <table class="text-sm w-full">
            <thead>
                <tr class="text-xs text-dimmed text-left">
                    <th class="font-normal py-1">{{ $t("whippyflightFlickAxis") }}</th>
                    <th class="font-normal">{{ $t("whippyflightFlickTargetLag") }}</th>
                    <th class="font-normal">{{ $t("whippyflightFlickControllerLag") }}</th>
                    <th class="font-normal">{{ $t("whippyflightFlickOvershoot") }}</th>
                    <th class="font-normal">{{ $t("whippyflightFlickRise90") }}</th>
                    <th class="font-normal">{{ $t("whippyflightFlickHoldErr") }}</th>
                    <th class="font-normal">{{ $t("whippyflightFlickSaturation") }}</th>
                    <th class="font-normal">{{ $t("whippyflightFlickRoughness") }}</th>
                </tr>
            </thead>
            <tbody>
                <tr
                    v-for="f in report.flicks"
                    :key="f.axis"
                    class="tabular-nums cursor-pointer hover:bg-elevated"
                    :class="{ 'bg-elevated font-semibold': f === flick }"
                    @click="selected = f.axis"
                >
                    <td class="py-1">{{ $t(`whippyflightAxis_${f.axis}`) }}, {{ degrees(f.amp) }}°/s</td>
                    <td>{{ f.metrics.target_lag_ms.toFixed(1) }} ms</td>
                    <td>{{ controllerLagMs(f.metrics).toFixed(1) }} ms</td>
                    <td>{{ percent(f.metrics.overshoot) }}</td>
                    <td>{{ f.metrics.rise90_ms === null ? "–" : `${f.metrics.rise90_ms.toFixed(1)} ms` }}</td>
                    <td>{{ percent(f.metrics.hold_err) }}</td>
                    <td :class="f.metrics.sat_frac > 0 ? 'text-warning' : ''">
                        {{ percent(f.metrics.sat_frac, false) }}
                    </td>
                    <td>{{ f.metrics.rough_v === null ? "–" : millivolts(f.metrics.rough_v) }}</td>
                </tr>
            </tbody>
        </table>

        <div v-if="flick" class="grid gap-3 grid-cols-1 lg:grid-cols-3">
            <FlickChart
                v-for="part in parts(flick)"
                :key="`${flick.axis}-${part.title}`"
                :title="part.title"
                :x-label="$t('whippyflightFlickTime')"
                :dt="report.trace_dt"
                :lines="lines(flick)"
                :saturated="flick.saturated"
                :span="part.span"
            />
            <FlickChart
                v-for="(part, i) in parts(flick)"
                :key="`${flick.axis}-motors-${part.title}`"
                :title="motorTitles[i]"
                :x-label="$t('whippyflightFlickTime')"
                :dt="report.trace_dt"
                :lines="motorLines(flick)"
                :saturated="flick.saturated"
                :span="part.span"
            />
        </div>
        <p class="text-xs text-dimmed">
            {{ report.hover_rough_v === null ? $t("whippyflightFlickNoNoise") : $t("whippyflightFlickRoughnessNote") }}
        </p>
        <p class="text-xs text-dimmed">
            {{ $t("whippyflightFlickNote", { fit: report.fit, interpolation: { escapeValue: false } }) }}
        </p>
    </div>
</template>

<script setup lang="ts">
import { computed, ref } from "vue";
import { useTranslation } from "i18next-vue";
import FlickChart, { type FlickLine } from "./FlickChart.vue";
import { controllerLagMs, type FlickAxis, type FlickReport, type FlickResult } from "@/js/whippyflight/flicks";

const props = defineProps<{ report: FlickReport }>();

const { t } = useTranslation();
const selected = ref<FlickAxis>("roll");
const flick = computed(() => props.report.flicks.find((f) => f.axis === selected.value) ?? props.report.flicks[0]);
const STICK_COLOR = "var(--chart-reference-color)";
const TARGET_COLOR = "var(--color-axis-yaw)";
const RESPONSE_COLOR = "var(--color-axis-pitch)";
const MOTOR_COLORS = [1, 2, 3, 4].map((n) => `var(--color-channel-${n})`);
/** Delay margin, ms, below which a slower gyro filter or loop jitter might upset the gains. */
const LOW_MARGIN_MS = 1.5;

const margin = computed(() => {
    const ms = props.report.delay_margin_ms;
    if (ms === null) {
        return "–";
    }
    if (ms < 0) {
        return t("whippyflightFlickMarginNone");
    }
    return `${ms >= props.report.max_margin_ms ? "≥ " : ""}${ms.toFixed(1)} ms`;
});
const marginLow = computed(() => props.report.delay_margin_ms !== null && props.report.delay_margin_ms < LOW_MARGIN_MS);
const motorTitles = computed(() => [
    t("whippyflightFlickMotors"),
    `${t("whippyflightFlickMotors")}, ${t("whippyflightFlickOnset").toLowerCase()}`,
    `${t("whippyflightFlickMotors")}, ${t("whippyflightFlickRelease").toLowerCase()}`,
]);

function lines(f: FlickResult): FlickLine[] {
    return [
        { values: f.stick, color: STICK_COLOR, label: t("whippyflightFlickStick"), dashed: true },
        { values: f.target, color: TARGET_COLOR, label: t("whippyflightFlickTarget") },
        { values: f.response, color: RESPONSE_COLOR, label: t("whippyflightFlickResponse"), width: 1.6 },
    ];
}

function motorLines(f: FlickResult): FlickLine[] {
    return f.volts.map((values, m) => ({
        values,
        color: MOTOR_COLORS[m],
        label: t("whippyflightFlickMotor", { n: m + 1 }),
    }));
}

/** The whole flick, then its onset and release up close (ms spans). */
function parts(f: FlickResult): { title: string; span?: [number, number] }[] {
    const title = t("whippyflightFlickTitle", { axis: t(`whippyflightAxis_${f.axis}`), dps: degrees(f.amp) });
    const release = f.rise_ms + 1e3 * props.report.hold_s;
    const near = 1.6 * f.rise_ms + 30;
    return [
        { title: `${title}, ${f.rise_ms.toFixed(0)} ms`, span: undefined },
        { title: t("whippyflightFlickOnset"), span: [-5, near] },
        { title: t("whippyflightFlickRelease"), span: [release - 5, release + near] },
    ];
}

function millivolts(volts: number, digits = 0): string {
    return `${(1e3 * volts).toFixed(digits)} mV`;
}

function degrees(rps: number): string {
    return (360 * rps).toFixed(0);
}

function percent(ratio: number, signed = true): string {
    const value = (100 * ratio).toFixed(1);
    return `${signed && ratio >= 0 ? "+" : ""}${value}%`;
}
</script>
