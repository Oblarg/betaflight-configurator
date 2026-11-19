<template>
    <div class="flex flex-col gap-2 text-sm">
        <div class="grid grid-cols-1 md:grid-cols-2 gap-x-6 gap-y-2">
            <div v-for="s in sliders" :key="s.key" class="flex flex-col gap-1" :title="s.hint">
                <div class="flex justify-between text-xs">
                    <span class="text-dimmed">{{ s.label }}</span>
                    <span class="font-mono text-dimmed">{{ s.symbol }} {{ format(weights[s.key]) }}</span>
                </div>
                <div class="flex items-center gap-2 text-xs">
                    <span class="w-10 text-right">{{ s.left }}</span>
                    <USlider
                        :model-value="sliderOf(weights[s.key], s.range)"
                        :min="0"
                        :max="SLIDER_MAX"
                        :step="1"
                        :disabled="disabled"
                        class="grow"
                        @update:model-value="(pos) => set(s.key, weightAt(sliderValue(pos), s.range))"
                        @dblclick="set(s.key, DEFAULT_WEIGHTS[s.key])"
                    />
                    <span class="w-10">{{ s.right }}</span>
                </div>
            </div>
        </div>
        <div class="flex items-center gap-2">
            <UButton
                size="xs"
                variant="link"
                color="neutral"
                :icon="advanced ? 'i-lucide-chevron-down' : 'i-lucide-chevron-right'"
                :label="$t('whippyflightWeightsAdvanced')"
                @click="advanced = !advanced"
            />
            <UButton
                v-if="!sameWeights(weights, defaults)"
                size="xs"
                variant="link"
                icon="i-lucide-rotate-ccw"
                :label="$t('whippyflightWeightsReset')"
                :disabled="disabled"
                @click="weights = { ...defaults }"
            />
            <span v-if="changed" class="text-xs text-warning">{{ $t("whippyflightWeightsChanged") }}</span>
        </div>
        <div v-if="advanced" class="flex flex-col gap-2 pl-2">
            <div class="flex flex-wrap gap-4">
                <label v-for="f in fields" :key="f.key" class="flex items-center gap-2 text-xs">
                    <span class="text-dimmed">{{ f.label }}</span>
                    <UInputNumber
                        :model-value="weights[f.key]"
                        :min="f.step"
                        :step="f.step"
                        :format-options="{ useGrouping: false, maximumSignificantDigits: 4 }"
                        :disabled="disabled"
                        size="xs"
                        class="w-28"
                        @update:model-value="(v) => v && v > 0 && set(f.key, v)"
                    />
                </label>
            </div>
            <p class="text-xs text-dimmed">{{ $t("whippyflightWeightsAdvancedHint") }}</p>
            <div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs">
                <label v-for="f in tauFields" :key="f.key" class="flex items-center gap-2">
                    <span class="text-dimmed">{{ f.label }}</span>
                    <UInputNumber
                        :model-value="weights[f.key] * f.scale"
                        :min="f.step"
                        :step="f.step"
                        :format-options="{ useGrouping: false, maximumSignificantDigits: 4 }"
                        :disabled="disabled"
                        size="xs"
                        class="w-28"
                        @update:model-value="(v) => v && v > 0 && set(f.key, v / f.scale)"
                    />
                </label>
                <UButton
                    v-if="!sameTimeConstants(weights, defaultTaus)"
                    size="xs"
                    variant="link"
                    icon="i-lucide-rotate-ccw"
                    :label="$t('whippyflightWeightsReset')"
                    :disabled="disabled"
                    @click="weights = { ...weights, ...defaultTaus }"
                />
            </div>
            <p class="text-xs text-dimmed">{{ $t("whippyflightTauHint") }}</p>
        </div>
    </div>
</template>

<script setup lang="ts">
import { computed, ref } from "vue";
import { useTranslation } from "i18next-vue";
import {
    DEFAULT_WEIGHTS,
    INTEGRAL_RANGE,
    sameTimeConstants,
    sameWeights,
    SLIDER_MAX,
    sliderOf,
    STIFFNESS_RANGE,
    weightAt,
    type TimeConstants,
    type WeightRange,
    type Weights,
} from "@/js/whippyflight/weights";

const props = defineProps<{
    /** The weights the current gains were computed with, if known. */
    recorded: Weights | null;
    /** The σ_rate, rot/s, and τ_deriv, s, the shown fit's front derived, if it has them. */
    derivedSigmaRate: number | null;
    derivedTauDeriv: number | null;
    disabled?: boolean;
}>();
const weights = defineModel<Weights>({ required: true });

const { t } = useTranslation();
const advanced = ref(false);

const changed = computed(() => props.recorded !== null && !sameWeights(weights.value, props.recorded));
const defaultTaus = computed<TimeConstants>(() => ({
    tauDeriv: props.derivedTauDeriv ?? DEFAULT_WEIGHTS.tauDeriv,
    tauInt: DEFAULT_WEIGHTS.tauInt,
}));
const defaults = computed<Weights>(() => ({
    ...DEFAULT_WEIGHTS,
    sigmaRate: props.derivedSigmaRate ?? DEFAULT_WEIGHTS.sigmaRate,
    ...defaultTaus.value,
}));

const tauFields = computed<{ key: keyof TimeConstants; label: string; scale: number; step: number }[]>(() => [
    { key: "tauDeriv", label: t("whippyflightTauDeriv"), scale: 1e3, step: 0.1 },
    { key: "tauInt", label: t("whippyflightTauInt"), scale: 1, step: 0.01 },
]);

const sliders = computed<
    {
        key: keyof Weights;
        label: string;
        hint: string;
        symbol: string;
        left: string;
        right: string;
        range: WeightRange;
    }[]
>(() => [
    {
        key: "sigmaDeriv",
        label: t("whippyflightWeightResponse"),
        hint: t("whippyflightWeightResponseHint"),
        symbol: "σ_deriv",
        left: t("whippyflightWeightStiff"),
        right: t("whippyflightWeightSmooth"),
        range: STIFFNESS_RANGE,
    },
    {
        key: "sigmaInt",
        label: t("whippyflightWeightIntegral"),
        hint: t("whippyflightWeightIntegralHint"),
        symbol: "σ_int",
        left: t("whippyflightWeightFirm"),
        right: t("whippyflightWeightLoose"),
        range: INTEGRAL_RANGE,
    },
]);

const fields = computed<{ key: keyof Weights; label: string; step: number }[]>(() => [
    { key: "sigmaRate", label: t("whippyflightSigmaRate"), step: 0.1 },
    { key: "sigmaDeriv", label: t("whippyflightSigmaDeriv"), step: 0.1 },
    { key: "sigmaInt", label: t("whippyflightSigmaInt"), step: 0.001 },
]);

function set(key: keyof Weights, value: number) {
    weights.value = { ...weights.value, [key]: value };
}

/** USlider emits an array for range sliders and may emit undefined. */
function sliderValue(pos: number | number[] | undefined): number {
    return Array.isArray(pos) ? pos[0] : (pos ?? 0);
}

function format(value: number): string {
    return String(Number(value.toPrecision(3)));
}
</script>
