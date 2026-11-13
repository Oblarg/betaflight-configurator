<template>
    <div class="flex flex-col gap-3">
        <ul v-if="diagnostics.warnings.length" class="flex flex-col gap-1">
            <li
                v-for="(warning, i) in diagnostics.warnings"
                :key="i"
                class="flex items-start gap-2 text-sm text-warning"
            >
                <UIcon name="i-lucide-triangle-alert" class="size-4 shrink-0 mt-0.5" />
                <span>{{ describeWarning(warning) }}</span>
            </li>
        </ul>
        <p v-else class="flex items-center gap-2 text-sm text-success">
            <UIcon name="i-lucide-circle-check" class="size-4" />
            {{ $t("whippyflightNoWarnings") }}
        </p>

        <table class="text-sm w-full">
            <thead>
                <tr class="text-xs text-dimmed text-left">
                    <th class="font-normal py-1">{{ $t("whippyflightAxis") }}</th>
                    <th class="font-normal">R²</th>
                    <th class="font-normal">
                        {{ $t("whippyflightHeldOutR2", { method: diagnostics.heldout_method }) }}
                    </th>
                    <th class="font-normal">{{ $t("whippyflightResidual") }}</th>
                    <th class="font-normal">{{ $t("whippyflightRows") }}</th>
                </tr>
            </thead>
            <tbody>
                <tr
                    v-for="axis in DIAGNOSTICS_AXES"
                    :key="axis"
                    class="cursor-pointer hover:bg-elevated"
                    :class="{ 'bg-elevated font-semibold': axis === selected }"
                    @click="selected = axis"
                >
                    <td class="py-1">{{ $t(`whippyflightAxis_${axis}`) }}</td>
                    <td>{{ diagnostics.axes[axis].r2.toFixed(3) }}</td>
                    <td>{{ diagnostics.axes[axis].heldout_r2?.toFixed(3) ?? "–" }}</td>
                    <td>
                        {{ format(diagnostics.axes[axis].residual_std) }} /
                        {{ format(diagnostics.axes[axis].measured_std) }}
                        {{ diagnostics.axes[axis].unit }}
                    </td>
                    <td>{{ diagnostics.axes[axis].rows }}</td>
                </tr>
            </tbody>
        </table>

        <div class="grid gap-3 grid-cols-1 lg:grid-cols-3">
            <DiagnosticChart
                v-if="axisData.excerpt"
                kind="series"
                :title="
                    $t(axisData.excerpt.window_s ? 'whippyflightChartExcerpt' : 'whippyflightChartExcerptWindow', {
                        flight: axisData.excerpt.flight,
                        seconds: axisData.excerpt.window_s,
                        interpolation: { escapeValue: false },
                    })
                "
                :x-label="$t('whippyflightExcerptSeconds')"
                :x="axisData.excerpt.t"
                :x-span="axisData.excerpt.window_s ? [0, axisData.excerpt.window_s] : undefined"
                :a="axisData.excerpt.measured"
                :b="axisData.excerpt.predicted"
            />
            <DiagnosticChart
                kind="scatter"
                :title="$t('whippyflightChartScatter')"
                :x-label="
                    $t('whippyflightMeasuredUnit', { unit: axisData.unit, interpolation: { escapeValue: false } })
                "
                :x="axisData.scatter.measured"
                :a="axisData.scatter.predicted"
            />
            <DiagnosticChart
                kind="histogram"
                :title="$t('whippyflightChartResiduals')"
                :x-label="
                    $t('whippyflightResidualUnit', { unit: axisData.unit, interpolation: { escapeValue: false } })
                "
                :x="axisData.residual_hist.edges"
                :a="axisData.residual_hist.counts"
            />
        </div>

        <div class="flex flex-wrap gap-x-5 gap-y-1 text-xs">
            <span v-for="c in axisData.coefficients" :key="c.name">
                <span class="text-dimmed">{{ c.name }}</span>
                {{ format(c.value) }}
                <span v-if="c.rel_se !== null" class="text-dimmed">± {{ (c.rel_se * 100).toFixed(1) }}%</span>
            </span>
        </div>

        <UiBox :title="$t('whippyflightFlights', { kept: diagnostics.kept_s })" type="neutral" collapsible>
            <table class="text-xs w-full">
                <thead>
                    <tr class="text-dimmed text-left">
                        <th class="font-normal py-1">{{ $t("whippyflightFlight") }}</th>
                        <th class="font-normal">{{ $t("whippyflightKept") }}</th>
                        <th v-for="axis in DIAGNOSTICS_AXES" :key="axis" class="font-normal">
                            R² {{ $t(`whippyflightAxis_${axis}`) }}
                        </th>
                        <th class="font-normal">{{ $t("whippyflightPairing") }}</th>
                    </tr>
                </thead>
                <tbody>
                    <tr v-for="flight in diagnostics.flights" :key="flight.label">
                        <td class="py-1">{{ flight.label }}</td>
                        <td>{{ flight.kept_s }} / {{ flight.duration_s }} s</td>
                        <td v-for="axis in DIAGNOSTICS_AXES" :key="axis">{{ flight.r2[axis].toFixed(3) }}</td>
                        <td :class="flight.pairing_mismatch.length ? 'text-warning' : 'text-success'">
                            {{ flight.pairing_mismatch.length ? flight.pairing_mismatch.join(", ") : "✓" }}
                        </td>
                    </tr>
                </tbody>
            </table>
        </UiBox>
    </div>
</template>

<script setup lang="ts">
import { computed, ref } from "vue";
import { useTranslation } from "i18next-vue";
import * as d3 from "d3";
import UiBox from "@/components/elements/UiBox.vue";
import DiagnosticChart from "./DiagnosticChart.vue";
import {
    DIAGNOSTICS_AXES,
    type Diagnostics,
    type DiagnosticsAxis,
    type DiagnosticsWarning,
} from "@/js/whippyflight/diagnostics";

const props = defineProps<{ diagnostics: Diagnostics }>();

const { t } = useTranslation();
const selected = ref<DiagnosticsAxis>("roll");
const axisData = computed(() => props.diagnostics.axes[selected.value]);
const format = d3.format(".3~g");

function describeWarning(warning: DiagnosticsWarning): string {
    const params: Record<string, unknown> = { ...warning };
    if ("axis" in warning) {
        params.axis = t(`whippyflightAxis_${warning.axis}`);
    }
    if (warning.code === "pairing_mismatch") {
        params.axes = warning.axes.map((axis) => t(`whippyflightAxis_${axis}`)).join(", ");
    }
    if (warning.code === "uncertain_coefficient") {
        params.rel_se = (warning.rel_se * 100).toFixed(1);
        params.limit = (warning.limit * 100).toFixed(0);
    }
    return t(`whippyflightWarning_${warning.code}`, params);
}
</script>
