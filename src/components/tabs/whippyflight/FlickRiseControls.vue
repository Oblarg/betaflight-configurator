<template>
    <div class="flex flex-wrap items-center gap-x-4 gap-y-2 text-xs">
        <label class="flex items-center gap-2" :title="$t('whippyflightFlickRiseHint')">
            <span class="text-dimmed">{{ $t("whippyflightFlickRise") }}</span>
            <UInputNumber
                :model-value="shown"
                :min="1"
                :max="MAX_FLICK_RISE_MS"
                :step="10"
                :format-options="{ useGrouping: false, maximumFractionDigits: 0 }"
                :disabled="disabled"
                size="xs"
                class="w-24"
                @update:model-value="set"
            />
            <span class="text-dimmed">ms</span>
        </label>
        <UButton
            v-if="rise !== null"
            size="xs"
            variant="link"
            icon="i-lucide-rotate-ccw"
            :label="$t('whippyflightFlickReset')"
            :disabled="disabled"
            @click="rise = null"
        />
    </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import { limitRiseMs, type FlickReport } from "@/js/whippyflight/flicks";
import { MAX_FLICK_RISE_MS } from "@/stores/whippyflight";

const props = defineProps<{
    /** The flicks flown last, whose fastest allowed stick the input shows until set. */
    report: FlickReport | null;
    disabled?: boolean;
}>();
/** Slowest stick rise set by hand, ms; `null` flies the fastest the setpoint filtering allows. */
const rise = defineModel<number | null>({ required: true });

const fastest = computed(() => {
    const ms = props.report?.flicks.length ? limitRiseMs(props.report) : Number.NaN;
    return Number.isFinite(ms) ? Math.min(Math.ceil(ms), MAX_FLICK_RISE_MS) : null;
});
const shown = computed(() => rise.value ?? fastest.value);

function set(ms: number | null | undefined) {
    if (!ms || ms <= 0 || ms > MAX_FLICK_RISE_MS || (rise.value === null && ms === fastest.value)) {
        return;
    }
    rise.value = ms;
}
</script>
