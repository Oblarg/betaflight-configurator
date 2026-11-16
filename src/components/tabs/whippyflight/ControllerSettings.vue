<template>
    <div class="flex flex-col gap-3 text-sm border-t border-default pt-2">
        <div v-if="checks.length" class="flex flex-col gap-1">
            <span class="text-xs text-dimmed">{{ $t("whippyflightChecksTitle") }}</span>
            <div v-for="item in checks" :key="item.check" class="flex items-start gap-2 text-xs">
                <UIcon name="i-lucide-circle-x" class="size-4 shrink-0 text-warning" />
                <span class="grow">{{ checkText(item) }}</span>
                <UButton
                    v-if="item.fix"
                    size="xs"
                    variant="outline"
                    :label="fixLabel(item)"
                    :disabled="saving"
                    @click="applyFix(item)"
                />
            </div>
        </div>

        <p v-if="loaded && !has.enabled" class="text-dimmed text-xs">{{ $t("whippyflightNoSettings") }}</p>
        <template v-else-if="loaded">
            <div class="grid grid-cols-[auto_auto_1fr] items-center gap-x-3 gap-y-2">
                <span class="text-dimmed">{{ $t("whippyflightEnabled") }}</span>
                <USwitch v-model="draft.enabled" size="sm" :disabled="saving" />
                <span class="text-xs text-dimmed">{{ $t("whippyflightEnabledHint") }}</span>

                <template v-if="has.motorLpfHz">
                    <span class="text-dimmed">{{ $t("whippyflightMotorSmoothing") }}</span>
                    <div class="flex items-center gap-1">
                        <UInputNumber
                            v-model="draft.motorLpfHz"
                            :min="0"
                            :max="infos.motorLpfHz?.max ?? undefined"
                            :step="5"
                            :format-options="{ useGrouping: false }"
                            :disabled="saving"
                            size="xs"
                            class="w-24"
                        />
                        <span class="text-xs text-dimmed w-6">{{
                            draft.motorLpfHz ? "Hz" : $t("whippyflightOff")
                        }}</span>
                    </div>
                    <span class="text-xs text-dimmed">{{ $t("whippyflightMotorSmoothingHint") }}</span>
                </template>

                <template v-if="has.stickTracking">
                    <span class="text-dimmed">{{ $t("whippyflightStickTracking") }}</span>
                    <UInputNumber
                        v-model="draft.stickTracking"
                        :min="lambdaOf(infos.stickTracking?.min) ?? 0.01"
                        :max="lambdaOf(infos.stickTracking?.max)"
                        :step="draft.stickTracking < 1 ? 0.05 : 1"
                        :step-snapping="false"
                        :format-options="{ useGrouping: false, maximumFractionDigits: 2 }"
                        :disabled="saving"
                        size="xs"
                        class="w-24"
                    />
                    <span class="text-xs text-dimmed">{{ $t("whippyflightStickTrackingHint") }}</span>
                </template>

                <template v-if="has.idleRps">
                    <span class="text-dimmed">{{ $t("whippyflightIdleSpeed") }}</span>
                    <div class="flex items-center gap-1">
                        <UInputNumber
                            v-model="draft.idleRps"
                            :min="0"
                            :max="infos.idleRps?.max ?? undefined"
                            :step="5"
                            :format-options="{ useGrouping: false }"
                            :disabled="saving"
                            size="xs"
                            class="w-24"
                        />
                        <span class="text-xs text-dimmed w-6">{{
                            draft.idleRps ? "rev/s" : $t("whippyflightOff")
                        }}</span>
                    </div>
                    <span class="text-xs text-dimmed">{{ $t("whippyflightIdleSpeedHint") }}</span>
                </template>

                <template v-if="has.motorPoles">
                    <span class="text-dimmed">{{ $t("whippyflightMotorPoles") }}</span>
                    <UInputNumber
                        v-model="draft.motorPoles"
                        :min="infos.motorPoles?.min ?? 4"
                        :max="infos.motorPoles?.max ?? undefined"
                        :step="2"
                        :format-options="{ useGrouping: false }"
                        :disabled="saving"
                        size="xs"
                        class="w-24"
                    />
                    <span class="text-xs text-dimmed">{{ $t("whippyflightMotorPolesHint") }}</span>
                </template>

                <template v-if="has.logTelemetry">
                    <span class="text-dimmed">{{ $t("whippyflightLogTelemetry") }}</span>
                    <USwitch v-model="draft.logTelemetry" size="sm" :disabled="saving" />
                    <span class="text-xs text-dimmed"
                        >{{ $t("whippyflightLogTelemetryHint") }}
                        <template v-if="otherDebugMode">{{
                            $t("whippyflightLogTelemetryReplaces", { mode: otherDebugMode })
                        }}</template></span
                    >
                </template>

                <template v-if="has.logAccel">
                    <span class="text-dimmed">{{ $t("whippyflightLogAccel") }}</span>
                    <USwitch v-model="draft.logAccel" size="sm" :disabled="saving" />
                    <span class="text-xs text-dimmed">{{ $t("whippyflightLogAccelHint") }}</span>
                </template>
            </div>
            <div class="flex items-center gap-3">
                <UButton
                    size="xs"
                    :label="$t(needsReboot ? 'whippyflightSaveReboot' : 'whippyflightSaveSettings')"
                    :disabled="!(changed.length || needsReboot) || saving"
                    :loading="saving"
                    @click="save"
                />
                <span v-if="!status.active" class="text-xs text-warning flex gap-1 items-start">
                    <UIcon name="i-lucide-triangle-alert" class="size-4 shrink-0" />
                    {{ $t("whippyflightInactive") }}
                </span>
            </div>
        </template>
        <p v-if="error" class="text-xs text-error">{{ error }}</p>
    </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { useTranslation } from "i18next-vue";
import { getSetting, getSettingInfo, setSetting, type SettingInfo } from "@/composables/useMspSetting";
import { useReboot } from "@/composables/useReboot";
import {
    ArmingCheck,
    CONTROLLER_SETTINGS,
    FIX_SETTINGS,
    STICK_TRACKING_SCALE,
    WHIPPYFLIGHT_DEBUG_MODE,
    failingChecks,
    type CheckItem,
    type ControllerSettingKey,
} from "@/js/whippyflight/fcSettings";
import type { WhippyFlightStatus } from "@/js/whippyflight/protocol";
import { useWhippyFlightStore } from "@/stores/whippyflight";

const props = defineProps<{ status: WhippyFlightStatus }>();

const { t } = useTranslation();
const { saveAndReboot, saveToEeprom } = useReboot();
const store = useWhippyFlightStore();

const KEYS = Object.keys(CONTROLLER_SETTINGS) as ControllerSettingKey[];

interface Draft {
    enabled: boolean;
    motorLpfHz: number;
    stickTracking: number;
    idleRps: number;
    motorPoles: number;
    logTelemetry: boolean;
    logAccel: boolean;
}

/** Settings shown as switches, which have no bounds to read. */
const SWITCHES: ControllerSettingKey[] = ["enabled", "logTelemetry", "logAccel"];

/** The FC's values as read, by firmware name; null for a setting this build lacks. */
const values = ref<Record<string, string | null>>({});
const infos = ref<Partial<Record<ControllerSettingKey, SettingInfo | null>>>({});
const loaded = ref(false);
const draft = reactive<Draft>({
    enabled: true,
    motorLpfHz: 0,
    stickTracking: 8,
    idleRps: 0,
    motorPoles: 14,
    logTelemetry: false,
    logAccel: true,
});
const saving = ref(false);
const error = ref("");

const has = computed(
    () =>
        Object.fromEntries(KEYS.map((k) => [k, values.value[CONTROLLER_SETTINGS[k].name] != null])) as Record<
            ControllerSettingKey,
            boolean
        >,
);

function stored(): Draft {
    const num = (k: ControllerSettingKey) => Number(values.value[CONTROLLER_SETTINGS[k].name] ?? 0);
    return {
        enabled: values.value[CONTROLLER_SETTINGS.enabled.name] === "ON",
        motorLpfHz: num("motorLpfHz"),
        stickTracking: num("stickTracking") / STICK_TRACKING_SCALE,
        idleRps: num("idleRps"),
        motorPoles: num("motorPoles"),
        logTelemetry: values.value[CONTROLLER_SETTINGS.logTelemetry.name] === WHIPPYFLIGHT_DEBUG_MODE,
        logAccel: values.value[CONTROLLER_SETTINGS.logAccel.name] === "OFF",
    };
}

/** The debug mode the FC logs instead, which turning telemetry on replaces; empty when none. */
const otherDebugMode = computed(() => {
    const mode = values.value[CONTROLLER_SETTINGS.logTelemetry.name];
    return mode && mode !== "NONE" && mode !== WHIPPYFLIGHT_DEBUG_MODE ? mode : "";
});

/** The draft as firmware values, by setting key. */
function asSettings(d: Draft): Record<ControllerSettingKey, string | number> {
    return {
        enabled: d.enabled ? "ON" : "OFF",
        motorLpfHz: d.motorLpfHz ?? 0,
        stickTracking: Math.round(d.stickTracking * STICK_TRACKING_SCALE),
        idleRps: d.idleRps ?? 0,
        motorPoles: d.motorPoles,
        logTelemetry: d.logTelemetry ? WHIPPYFLIGHT_DEBUG_MODE : otherDebugMode.value || "NONE",
        logAccel: d.logAccel ? "OFF" : "ON",
    };
}

const changed = computed(() => {
    const now = asSettings(draft);
    const was = asSettings(stored());
    return KEYS.filter((k) => has.value[k] && now[k] !== was[k]);
});

/** The controller is chosen at boot, so a change to it (saved or not) waits for a reboot. */
const needsReboot = computed(
    () =>
        (has.value.enabled && draft.enabled !== props.status.active) ||
        changed.value.some((k) => CONTROLLER_SETTINGS[k].reboot),
);

const checks = computed(() => failingChecks(props.status, values.value));

/** A `whippyflight_stick_tracking` bound as λ. */
function lambdaOf(setting: number | null | undefined): number | undefined {
    return setting == null ? undefined : setting / STICK_TRACKING_SCALE;
}

async function load(): Promise<void> {
    const names = [...KEYS.map((k) => CONTROLLER_SETTINGS[k].name), ...FIX_SETTINGS];
    const read: Record<string, string | null> = {};
    for (const name of names) {
        read[name] = await getSetting(name);
    }
    values.value = read;
    const info: Partial<Record<ControllerSettingKey, SettingInfo | null>> = {};
    for (const k of KEYS) {
        if (!SWITCHES.includes(k) && read[CONTROLLER_SETTINGS[k].name] != null) {
            info[k] = await getSettingInfo(CONTROLLER_SETTINGS[k].name);
        }
    }
    infos.value = info;
    Object.assign(draft, stored());
    loaded.value = true;
}

/** Write `changes` by firmware name, then save them, rebooting when one only takes effect at boot. */
async function write(changes: [string, string | number][], reboot: boolean): Promise<void> {
    saving.value = true;
    error.value = "";
    try {
        for (const [name, value] of changes) {
            await setSetting(name, value);
        }
        if (reboot) {
            await saveAndReboot();
        } else {
            await saveToEeprom();
            await load();
        }
    } catch (e) {
        error.value = e instanceof Error ? e.message : String(e);
    } finally {
        saving.value = false;
    }
}

function save(): Promise<void> {
    const now = asSettings(draft);
    return write(
        changed.value.map((k) => [CONTROLLER_SETTINGS[k].name, now[k]]),
        needsReboot.value,
    );
}

function applyFix(item: CheckItem): Promise<void> {
    const fix = item.fix!;
    return write([[fix.setting, fix.value]], fix.reboot);
}

function hz(dt: number): string {
    return dt > 0 ? `${Math.round(1 / dt)} Hz` : "–";
}

function checkText(item: CheckItem): string {
    switch (item.check) {
        case ArmingCheck.GAINS:
            return t("whippyflightCheck_GAINS");
        case ArmingCheck.LOOP_RATE:
            return t("whippyflightCheck_LOOP_RATE", { pid: hz(props.status.pidDt), gains: hz(props.status.blobDt) });
        case ArmingCheck.RPM_TELEMETRY:
            return t("whippyflightCheck_RPM_TELEMETRY");
        case ArmingCheck.SELF_LEVEL:
            return t("whippyflightCheck_SELF_LEVEL");
        default:
            return t("whippyflightCheck_FAILSAFE");
    }
}

function fixLabel(item: CheckItem): string {
    const fix = item.fix!;
    return t(fix.reboot ? "whippyflightFixReboot" : "whippyflightFix", {
        change: `${fix.setting} = ${fix.value}`,
        interpolation: { escapeValue: false },
    });
}

// The flicks fly the stick tracking entered here, so its effect shows before it is saved.
watch(
    () => (loaded.value && has.value.stickTracking && draft.stickTracking > 0 ? draft.stickTracking : null),
    (lambda) => (store.stickTracking = lambda),
    { immediate: true },
);

// So is its motor-speed low-pass.
watch(
    () =>
        loaded.value && has.value.motorLpfHz && Number.isFinite(draft.motorLpfHz)
            ? Math.max(0, draft.motorLpfHz)
            : null,
    (hz) => (store.motorLpfHz = hz),
    { immediate: true },
);

// Gains are computed for the idle speed entered here: their grid starts there.
watch(
    () => (loaded.value && has.value.idleRps && Number.isFinite(draft.idleRps) ? Math.max(0, draft.idleRps) : null),
    (rps) => (store.idleRps = rps),
    { immediate: true },
);

onMounted(() => {
    load().catch((e: unknown) => {
        error.value = e instanceof Error ? e.message : String(e);
    });
});

onUnmounted(() => {
    store.stickTracking = null;
    store.motorLpfHz = null;
    store.idleRps = null;
});
</script>
