<template>
    <div class="flex w-full flex-col gap-2 text-xs">
        <div class="flex items-center gap-2">
            <UButton
                v-for="g in GROUPS"
                :key="g"
                size="xs"
                variant="link"
                color="neutral"
                :icon="open[g] ? 'i-lucide-chevron-down' : 'i-lucide-chevron-right'"
                :title="$t(`whippyflightFlick_${g}Hint`)"
                @click="open[g] = !open[g]"
            >
                {{ $t(`whippyflightFlick_${g}`) }}
                <span v-if="changed[g]" class="size-1.5 rounded-full bg-warning" />
            </UButton>
            <UButton
                v-if="!isDefault"
                size="xs"
                variant="link"
                icon="i-lucide-rotate-ccw"
                :label="$t('whippyflightFlickReset')"
                :disabled="disabled"
                @click="model = { ...DEFAULT_FLICK_CONDITIONS, cg_shift_pct: [0, 0] }"
            />
        </div>
        <div v-if="open.sensor" class="flex flex-wrap items-center gap-x-6 gap-y-2 pl-2">
            <label class="flex items-center gap-2" :title="$t('whippyflightFlickGyroFilterHint')">
                <span class="text-dimmed">{{ $t("whippyflightFlickGyroFilter") }}</span>
                <USwitch v-model="model.gyro_filter" size="sm" :disabled="disabled" />
            </label>

            <div class="flex items-center gap-2" :title="$t('whippyflightFlickDelayHint')">
                <span class="text-dimmed">{{ $t("whippyflightFlickDelay") }}</span>
                <label v-for="c in DELAYS" :key="c.key" class="flex items-center gap-1">
                    <span class="text-dimmed">{{ $t(c.label) }}</span>
                    <UInputNumber
                        :model-value="model[c.key] ?? loggedLatency?.[c.logged] ?? 0"
                        :min="0"
                        :max="MAX_FLICK_DELAY_MS"
                        :step="0.1"
                        :format-options="{ useGrouping: false, maximumFractionDigits: 2 }"
                        :disabled="disabled"
                        size="xs"
                        class="w-20"
                        @update:model-value="(v) => setDelay(c.key, v)"
                    />
                    <span class="text-dimmed">ms</span>
                </label>
                <span v-if="delaysAsFitted" class="text-dimmed"
                    >({{ $t(loggedLatency ? "whippyflightFlickDelayFitted" : "whippyflightFlickDelayNone") }})</span
                >
                <UButton
                    v-else
                    size="xs"
                    variant="link"
                    icon="i-lucide-rotate-ccw"
                    :label="$t('whippyflightFlickDelayFromFit')"
                    :disabled="disabled"
                    @click="delaysFromFit"
                />
            </div>

            <div class="flex items-center gap-2" :title="$t('whippyflightFlickNoiseHint')">
                <span class="text-dimmed"
                    >{{ $t("whippyflightFlickNoise") }} {{ $t("whippyflightFlickNoiseRms") }}</span
                >
                <template v-if="loggedNoise">
                    <div v-for="c in NOISE_CHANNELS" :key="c.key" class="flex items-center gap-1">
                        <UCheckbox
                            v-model="model[c.on]"
                            size="sm"
                            :label="$t(c.label)"
                            :disabled="disabled"
                            :ui="{ label: 'text-xs text-dimmed' }"
                        />
                        <UInputNumber
                            :model-value="model[c.key] ?? loggedNoise[c.logged]"
                            :min="0"
                            :step="c.step"
                            :format-options="{ useGrouping: false, maximumFractionDigits: c.digits }"
                            :disabled="disabled || !model[c.on]"
                            size="xs"
                            class="w-24"
                            @update:model-value="(v) => setNoise(c.key, v)"
                        />
                        <span class="text-dimmed">{{ c.unit }}</span>
                        <span
                            v-if="c.key === 'motor_speed_noise_rms' && loggedNoise.motor_speed_exponent !== null"
                            class="text-dimmed"
                            :title="$t('whippyflightFlickNoiseSpeedHint')"
                        >
                            {{
                                $t("whippyflightFlickNoiseSpeed", {
                                    k: loggedNoise.motor_speed_exponent.toFixed(2),
                                })
                            }}
                        </span>
                    </div>
                    <span v-if="noiseAsLogged" class="text-dimmed">({{ $t("whippyflightFlickNoiseLogged") }})</span>
                    <UButton
                        v-else
                        size="xs"
                        variant="link"
                        icon="i-lucide-rotate-ccw"
                        :label="$t('whippyflightFlickNoiseFromLogs')"
                        :disabled="disabled"
                        @click="noiseFromLogs"
                    />
                </template>
                <span v-else class="text-dimmed">{{ $t("whippyflightFlickNoiseNone") }}</span>
                <label class="flex items-center gap-1" :title="$t('whippyflightFlickNoisyFlightsHint')">
                    <span class="text-dimmed">{{ $t("whippyflightFlickNoisyFlights") }}</span>
                    <USwitch v-model="model.noisy_flights" size="sm" :disabled="disabled" />
                </label>
            </div>
        </div>
        <div v-if="open.system" class="flex flex-wrap items-center gap-x-6 gap-y-2 pl-2">
            <div class="flex items-center gap-2" :title="$t('whippyflightFlickCgHint')">
                <span class="text-dimmed">{{ $t("whippyflightFlickCg") }}</span>
                <label v-for="(dir, i) in CG_DIRS" :key="dir" class="flex items-center gap-1">
                    <span class="text-dimmed">{{ $t(`whippyflightFlickCg_${dir}`) }}</span>
                    <UInputNumber
                        :model-value="model.cg_shift_pct[i]"
                        :min="-50"
                        :max="50"
                        :step="1"
                        :format-options="{ maximumFractionDigits: 1 }"
                        :disabled="disabled"
                        size="xs"
                        class="w-20"
                        @update:model-value="(v) => setCg(i, v)"
                    />
                    <span class="text-dimmed">%</span>
                </label>
            </div>

            <div class="flex items-center gap-2" :title="$t('whippyflightFlickTurbulenceHint')">
                <UCheckbox
                    v-model="model.turbulence"
                    size="sm"
                    :label="`${$t('whippyflightFlickTurbulence')} ${$t('whippyflightFlickNoiseRms')}`"
                    :disabled="disabled"
                    :ui="{ label: 'text-xs text-dimmed' }"
                />
                <label v-for="(axis, i) in AXES" :key="axis" class="flex items-center gap-1">
                    <span class="text-dimmed">{{ $t(`whippyflightAxis_${axis}`) }}</span>
                    <UInputNumber
                        :model-value="turbulenceRms[i]"
                        :min="0"
                        :step="0.1"
                        :format-options="{ useGrouping: false, maximumFractionDigits: 2 }"
                        :disabled="disabled || !model.turbulence"
                        size="xs"
                        class="w-24"
                        @update:model-value="(v) => setTurbulenceRms(i, v)"
                    />
                </label>
                <span class="text-dimmed">rot/s²</span>
                <UInputNumber
                    :model-value="turbulenceHz"
                    :min="0.1"
                    :max="1000"
                    :step="0.5"
                    :format-options="{ useGrouping: false, maximumFractionDigits: 1 }"
                    :disabled="disabled || !model.turbulence"
                    size="xs"
                    class="w-20"
                    @update:model-value="setTurbulenceHz"
                />
                <span class="text-dimmed">Hz</span>
                <template v-if="loggedTurbulence">
                    <span v-if="turbulenceAsLogged" class="text-dimmed"
                        >({{ $t("whippyflightFlickNoiseLogged") }})</span
                    >
                    <UButton
                        v-else
                        size="xs"
                        variant="link"
                        icon="i-lucide-rotate-ccw"
                        :label="$t('whippyflightFlickNoiseFromLogs')"
                        :disabled="disabled"
                        @click="turbulenceFromLogs"
                    />
                </template>
                <span v-else class="text-dimmed">{{ $t("whippyflightFlickTurbulenceNone") }}</span>
            </div>
        </div>
    </div>
</template>

<script setup lang="ts">
import { computed, reactive } from "vue";
import {
    DEFAULT_FLICK_CONDITIONS,
    MAX_FLICK_DELAY_MS,
    sameConditions,
    type FlickConditions,
    type Latency,
    type NoiseRms,
    type TurbulenceSpec,
} from "@/js/whippyflight/flicks";

const CG_DIRS = ["right", "forward"] as const;
const AXES = ["roll", "pitch", "yaw"] as const;
const GROUPS = ["sensor", "system"] as const;
const SENSOR_KEYS = [
    "gyro_filter",
    "gyro_noise",
    "motor_noise",
    "gyro_noise_rms",
    "motor_speed_noise_rms",
    "noisy_flights",
    "actuator_delay_ms",
    "telemetry_delay_ms",
] as const;
const DELAYS = [
    { key: "actuator_delay_ms", logged: "actuator_ms", label: "whippyflightFlickDelayActuator" },
    { key: "telemetry_delay_ms", logged: "telemetry_ms", label: "whippyflightFlickDelayTelemetry" },
] as const;
const NOISE_CHANNELS = [
    {
        key: "gyro_noise_rms",
        on: "gyro_noise",
        logged: "gyro",
        label: "whippyflightFlickNoiseGyro",
        unit: "rot/s",
        step: 0.001,
        digits: 4,
    },
    {
        key: "motor_speed_noise_rms",
        on: "motor_noise",
        logged: "motor",
        label: "whippyflightFlickNoiseMotor",
        unit: "rev/s",
        step: 0.1,
        digits: 2,
    },
] as const;
/** What the turbulence fields show without a setting or a measurement. */
const UNMEASURED: TurbulenceSpec = { rms: [0, 0, 0], hz: 10 };

const props = defineProps<{
    /** The gains fit's sensor noise as logged; null without it. */
    loggedNoise: NoiseRms | null;
    /** The turbulence the gains fit's diagnostics measured; null without it. */
    loggedTurbulence: TurbulenceSpec | null;
    /** The latency the gains fit measured; null without it. */
    loggedLatency: Latency | null;
    disabled?: boolean;
}>();
const model = defineModel<FlickConditions>({ required: true });

const open = reactive({ sensor: false, system: false });

const isDefault = computed(() => sameConditions(model.value, DEFAULT_FLICK_CONDITIONS));
const changed = computed(() => {
    const m = model.value;
    const d = DEFAULT_FLICK_CONDITIONS;
    return {
        sensor: SENSOR_KEYS.some((k) => m[k] !== d[k]),
        system:
            m.cg_shift_pct.some((pct, i) => pct !== d.cg_shift_pct[i]) ||
            m.turbulence !== d.turbulence ||
            !turbulenceAsLogged.value,
    };
});

const noiseAsLogged = computed(() => model.value.gyro_noise_rms === null && model.value.motor_speed_noise_rms === null);
const turbulenceAsLogged = computed(() => model.value.turbulence_rms === null && model.value.turbulence_hz === null);
const delaysAsFitted = computed(
    () => model.value.actuator_delay_ms === null && model.value.telemetry_delay_ms === null,
);
const turbulenceRms = computed(() => model.value.turbulence_rms ?? props.loggedTurbulence?.rms ?? UNMEASURED.rms);
const turbulenceHz = computed(() => model.value.turbulence_hz ?? props.loggedTurbulence?.hz ?? UNMEASURED.hz);

function valid(x: number | null | undefined): x is number {
    return x !== null && x !== undefined && Number.isFinite(x);
}

function setNoise(key: (typeof NOISE_CHANNELS)[number]["key"], rms: number | null | undefined) {
    if (valid(rms) && rms >= 0) {
        model.value[key] = rms;
    }
}

function noiseFromLogs() {
    model.value.gyro_noise_rms = null;
    model.value.motor_speed_noise_rms = null;
}

function setDelay(key: (typeof DELAYS)[number]["key"], ms: number | null | undefined) {
    if (valid(ms) && ms >= 0 && ms <= MAX_FLICK_DELAY_MS) {
        model.value[key] = ms;
    }
}

function delaysFromFit() {
    model.value.actuator_delay_ms = null;
    model.value.telemetry_delay_ms = null;
}

function setTurbulenceRms(i: number, rms: number | null | undefined) {
    if (valid(rms) && rms >= 0) {
        const next: [number, number, number] = [...turbulenceRms.value];
        next[i] = rms;
        model.value.turbulence_rms = next;
    }
}

function setTurbulenceHz(hz: number | null | undefined) {
    if (valid(hz) && hz > 0) {
        model.value.turbulence_hz = hz;
    }
}

function turbulenceFromLogs() {
    model.value.turbulence_rms = null;
    model.value.turbulence_hz = null;
}

function setCg(i: number, pct: number | null | undefined) {
    if (valid(pct)) {
        const next: [number, number] = [...model.value.cg_shift_pct];
        next[i] = pct;
        model.value.cg_shift_pct = next;
    }
}
</script>
