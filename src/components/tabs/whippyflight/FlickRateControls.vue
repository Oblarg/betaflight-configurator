<template>
    <div class="flex flex-wrap items-center gap-x-4 gap-y-2 text-xs">
        <span class="text-dimmed" :title="$t('whippyflightFlickRatesHint')">{{ $t("whippyflightFlickRates") }}</span>
        <label v-for="(axis, i) in AXES" :key="axis" class="flex items-center gap-2">
            <span class="text-dimmed">{{ $t(`whippyflightAxis_${axis}`) }}</span>
            <UInputNumber
                :model-value="shown?.[i]"
                :min="0.01"
                :max="6"
                :step="0.1"
                :format-options="{ useGrouping: false, maximumFractionDigits: 2 }"
                :disabled="disabled"
                size="xs"
                class="w-24"
                @update:model-value="(v) => v && v > 0 && set(i, v)"
            />
            <span class="text-dimmed">rot/s</span>
        </label>
        <UButton
            v-if="rates"
            size="xs"
            variant="link"
            icon="i-lucide-rotate-ccw"
            :label="$t('whippyflightFlickReset')"
            :disabled="disabled"
            @click="rates = null"
        />
    </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import { flownRates, type FlickReport, type ManualFlickRates } from "@/js/whippyflight/flicks";

const AXES = ["roll", "pitch", "yaw"] as const;
/** Betaflight's default full-stick rate, rot/s. */
const DEFAULT_RPS = 670 / 360;

const props = defineProps<{
    /** The flicks flown last, whose rates the inputs show until set. */
    report: FlickReport | null;
    disabled?: boolean;
}>();
/** Rates set by hand; `null` flies the default ones (the FC's, else the logs'). */
const rates = defineModel<ManualFlickRates | null>({ required: true });

const flown = computed(() => {
    const rps = props.report ? flownRates(props.report) : null;
    return rps?.every(Number.isFinite) ? rps : null;
});
const shown = computed(() => rates.value ?? flown.value);

function set(axis: number, rps: number) {
    const next = [...(shown.value ?? [DEFAULT_RPS, DEFAULT_RPS, DEFAULT_RPS])] as ManualFlickRates;
    if (next[axis] === rps && !rates.value) {
        return;
    }
    next[axis] = rps;
    rates.value = next;
}
</script>
