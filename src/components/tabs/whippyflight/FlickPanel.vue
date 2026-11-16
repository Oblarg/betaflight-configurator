<template>
    <div class="flex flex-col gap-3">
        <p v-if="report.rates_from === 'default'" class="text-sm text-warning">
            {{ $t("whippyflightFlickDefaultRates") }}
        </p>
        <p v-if="zeroed.length" class="text-sm text-warning">
            {{
                zeroed.length === FEEDBACK_TERMS.length
                    ? $t("whippyflightFlickFeedforwardOnlyFlown")
                    : $t("whippyflightFlickFeedbackFlown", { terms: zeroed.join(", ") })
            }}
        </p>
        <p v-if="report.conditions.gyro_filter && !report.gyro_filtered" class="text-sm text-warning">
            {{ $t("whippyflightFlickGyroFilterUnmodeled") }}
        </p>

        <div class="flex flex-wrap gap-x-6 gap-y-1 text-sm">
            <span
                class="flex gap-1"
                :title="$t('whippyflightFlickMarginHint', { max: report.max_margin_ms.toFixed(0) })"
            >
                <span class="text-dimmed">{{ $t("whippyflightFlickMargin") }}</span>
                <span class="tabular-nums" :class="marginLow ? 'text-warning' : ''">{{ margin }}</span>
            </span>
            <span class="flex gap-1" :title="$t('whippyflightFlickDelayHint')">
                <span class="text-dimmed">{{ $t("whippyflightFlickDelay") }}</span>
                <span class="tabular-nums">{{
                    $t("whippyflightFlickDelayFlown", {
                        actuator: report.latency.actuator_ms.toFixed(2),
                        telemetry: report.latency.telemetry_ms.toFixed(2),
                    })
                }}</span>
                <span class="text-dimmed">{{
                    $t(delaysSet ? "whippyflightFlickDelaySetShort" : "whippyflightFlickDelayFitShort")
                }}</span>
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
            <span class="flex gap-1" :title="$t('whippyflightFlickMotorLpfHint')">
                <span class="text-dimmed">{{ $t("whippyflightFlickMotorLpf") }}</span>
                <span class="tabular-nums">{{
                    report.motor_lpf_hz ? `${report.motor_lpf_hz} Hz` : $t("whippyflightOff")
                }}</span>
                <span class="text-dimmed">{{
                    $t(
                        report.motor_lpf_from === "fc"
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
                    <th class="font-normal" :title="$t('whippyflightFlickStickRiseHint')">
                        {{ $t("whippyflightFlickStickRise") }}
                    </th>
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
                    :key="key(f)"
                    class="tabular-nums cursor-pointer hover:bg-elevated"
                    :class="{ 'bg-elevated font-semibold': f === flick }"
                    @click="selected = key(f)"
                >
                    <td class="py-1">{{ direction(f) }}, {{ rate(f.amp) }} rot/s</td>
                    <td :class="f.rise_ms > f.limit_rise_ms + 0.5 ? '' : 'text-dimmed'">
                        {{ f.rise_ms.toFixed(0) }} ms
                    </td>
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
                :key="`${key(flick)}-${part.title}`"
                :title="part.title"
                :x-label="$t('whippyflightFlickTime')"
                :dt="report.trace_dt"
                :lines="lines(flick)"
                :saturated="flick.saturated"
                :span="part.span"
            />
            <template v-for="row in motorRows" :key="row.name">
                <FlickChart
                    v-for="(part, i) in parts(flick)"
                    :key="`${key(flick)}-${row.name}-${part.title}`"
                    :title="partTitle(row.name, i)"
                    :x-label="$t('whippyflightFlickTime')"
                    :dt="report.trace_dt"
                    :lines="row.lines"
                    :saturated="flick.saturated"
                    :span="part.span"
                />
            </template>
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
import { controllerLagMs, FEEDBACK_TERMS, type FlickReport, type FlickResult } from "@/js/whippyflight/flicks";

const props = defineProps<{ report: FlickReport }>();

const { t } = useTranslation();
const selected = ref("roll+");
const flick = computed(() => props.report.flicks.find((f) => key(f) === selected.value) ?? props.report.flicks[0]);
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
/** Whether the conditions set either delay rather than the fit. */
const delaysSet = computed(
    () => props.report.conditions.actuator_delay_ms !== null || props.report.conditions.telemetry_delay_ms !== null,
);
/** The feedback gains zeroed for these flights, as P, I, D. */
const zeroed = computed(() => FEEDBACK_TERMS.filter((t) => !props.report.feedback[t]).map((t) => t.toUpperCase()));
const marginLow = computed(() => props.report.delay_margin_ms !== null && props.report.delay_margin_ms < LOW_MARGIN_MS);
/** The per-motor charts under the response: voltages, then shaft speeds. */
const motorRows = computed(() => {
    const f = flick.value;
    return [
        { name: t("whippyflightFlickMotors"), lines: motorLines(f.volts) },
        { name: t("whippyflightFlickMotorSpeeds"), lines: motorLines(f.motor_speeds) },
    ];
});

/** `name` for the whole flick, its onset (1) or its release (2). */
function partTitle(name: string, part: number): string {
    const parts = ["", "whippyflightFlickOnset", "whippyflightFlickRelease"];
    return part ? `${name}, ${t(parts[part]).toLowerCase()}` : name;
}

/** Which way `f` went, as an axis and a sign: roll right, pitch up, yaw right are positive in FRD. */
function key(f: FlickResult): string {
    return `${f.axis}${f.amp < 0 ? "-" : "+"}`;
}

function direction(f: FlickResult): string {
    return t(`whippyflightFlickDir_${f.axis}_${f.amp < 0 ? "neg" : "pos"}`);
}

function lines(f: FlickResult): FlickLine[] {
    return [
        { values: f.stick, color: STICK_COLOR, label: t("whippyflightFlickStick"), dashed: true },
        { values: f.target, color: TARGET_COLOR, label: t("whippyflightFlickTarget") },
        { values: f.response, color: RESPONSE_COLOR, label: t("whippyflightFlickResponse"), width: 1.6 },
    ];
}

function motorLines(motors: number[][]): FlickLine[] {
    return motors.map((values, m) => ({
        values,
        color: MOTOR_COLORS[m],
        label: t("whippyflightFlickMotor", { n: m + 1 }),
    }));
}

/** The whole flick, then its onset and release up close (ms spans). */
function parts(f: FlickResult): { title: string; span?: [number, number] }[] {
    const title = t("whippyflightFlickTitle", { axis: direction(f), rps: rate(f.amp) });
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

function rate(rps: number): string {
    return Math.abs(rps).toFixed(2);
}

function percent(ratio: number, signed = true): string {
    const value = (100 * ratio).toFixed(1);
    return `${signed && ratio >= 0 ? "+" : ""}${value}%`;
}
</script>
