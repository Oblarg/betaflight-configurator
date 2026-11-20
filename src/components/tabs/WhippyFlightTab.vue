<template>
    <BaseTab tab-name="whippyflight">
        <div class="content_wrapper flex flex-col gap-4">
            <div class="tab_title">{{ $t("tabWhippyFlight") }}</div>

            <UiBox :title="$t('whippyflightFlightController')" type="neutral">
                <div class="flex items-start justify-between gap-4">
                    <div class="flex flex-col gap-2 min-w-0">
                        <template v-if="connected && store.status">
                            <GainStatus :status="store.status" />
                            <ControllerSettings :status="store.status" />
                        </template>
                        <p v-else-if="connected" class="text-dimmed text-sm">{{ $t("whippyflightNoStatus") }}</p>
                        <p v-else class="text-dimmed text-sm">{{ $t("whippyflightOffline") }}</p>
                    </div>
                    <UButton
                        size="xs"
                        variant="outline"
                        icon="i-lucide-refresh-cw"
                        :disabled="!!store.busy"
                        @click="refreshAll"
                    />
                </div>
            </UiBox>

            <p v-if="!pipeline.isDesktop" class="text-warning text-sm flex gap-2 items-start">
                <UIcon name="i-lucide-monitor" class="size-4 shrink-0 mt-0.5" />
                {{ $t("whippyflightDesktopOnly") }}
            </p>

            <template v-else>
                <!-- Setup: toolchain and working directory -->
                <UiBox :title="$t('whippyflightStepSetup')" :type="boxType('setup')" :highlight="isCurrent('setup')">
                    <div class="grid grid-cols-[9rem_1fr_auto] items-center gap-2 text-sm">
                        <span class="text-dimmed">{{ $t("whippyflightPython") }}</span>
                        <UInput v-model="store.settings.python" size="xs" class="font-mono" @change="checkEnv" />
                        <UButton
                            size="xs"
                            :variant="pipeline.toolchainReady.value ? 'outline' : 'solid'"
                            :label="$t('whippyflightSetupPython')"
                            :title="$t('whippyflightSetupPythonHint')"
                            :disabled="!!store.busy && store.busy !== 'setup'"
                            :loading="store.busy === 'setup'"
                            @click="pipeline.setupPython"
                        />

                        <span class="text-dimmed">{{ $t("whippyflightWorkdir") }}</span>
                        <span class="font-mono text-xs truncate">{{ store.settings.workdir || "–" }}</span>
                        <div class="flex gap-1">
                            <UButton
                                size="xs"
                                variant="outline"
                                :label="$t('whippyflightChoose')"
                                @click="pipeline.chooseWorkdir"
                            />
                            <UButton
                                v-if="store.settings.workdir"
                                size="xs"
                                variant="outline"
                                icon="i-lucide-folder-open"
                                @click="pipeline.revealWorkdir()"
                            />
                        </div>
                    </div>
                    <div v-if="store.env" class="text-xs flex flex-col gap-0.5">
                        <span v-if="!store.env.pythonVersion" class="text-error">
                            {{ $t("whippyflightPythonMissing") }}
                        </span>
                        <span v-else-if="store.env.missingModules.length" class="text-error">
                            {{
                                $t("whippyflightModulesMissing", {
                                    version: store.env.pythonVersion,
                                    modules: store.env.missingModules.join(", "),
                                    interpolation: { escapeValue: false },
                                })
                            }}
                        </span>
                        <span v-else class="text-success">
                            {{ $t("whippyflightPythonFound", { version: store.env.pythonVersion }) }}
                        </span>
                        <span v-if="store.env.pythonVersion && !store.env.hasMatplotlib" class="text-dimmed">
                            {{ $t("whippyflightNoMatplotlib") }}
                        </span>
                    </div>
                </UiBox>

                <!-- Flight logs -->
                <UiBox :title="$t('whippyflightStepLogs')" :type="boxType('logs')" :highlight="isCurrent('logs')">
                    <div class="flex flex-wrap items-center justify-between gap-4 text-sm">
                        <span v-if="connected">
                            {{ $t("whippyflightFlashUsed", { size: megabytes(pipeline.flashUsed.value) }) }}
                        </span>
                        <span v-else class="text-dimmed">{{ $t("whippyflightImportHint") }}</span>
                        <div class="flex flex-wrap gap-2">
                            <UButton
                                size="xs"
                                variant="outline"
                                icon="i-lucide-file-plus"
                                :label="$t('whippyflightImportLogs')"
                                :disabled="!canImport"
                                :loading="store.busy === 'import'"
                                @click="pipeline.importLogs(false)"
                            />
                            <UButton
                                size="xs"
                                variant="outline"
                                icon="i-lucide-file-cog"
                                :label="$t('whippyflightImportDump')"
                                :disabled="!canImport"
                                @click="pipeline.importLogs(true)"
                            />
                            <UButton
                                v-if="connected"
                                size="xs"
                                icon="i-lucide-download"
                                :label="$t('whippyflightDownloadLogs')"
                                :disabled="!canWork || !pipeline.flashUsed.value"
                                :loading="store.busy === 'pull'"
                                @click="pipeline.pullLogs"
                            />
                            <UButton
                                v-if="connected"
                                size="xs"
                                color="error"
                                variant="outline"
                                icon="i-lucide-trash-2"
                                :label="$t('whippyflightEraseLogs')"
                                :disabled="!!store.busy || !pipeline.flashUsed.value"
                                :loading="store.busy === 'erase'"
                                @click="pipeline.eraseFlash"
                            />
                        </div>
                    </div>
                    <UProgress
                        v-if="store.busy === 'pull' && store.progress !== null"
                        :model-value="store.progress * 100"
                    />
                    <FlightPicker
                        v-if="store.workspace?.logs.length"
                        v-model="store.selectedFlights"
                        :logs="store.workspace.logs"
                        :flights="pipeline.flights.value"
                        :index="store.flightIndex"
                        :unscanned="pipeline.unscanned.value"
                        :disabled="!canWork"
                        @remove-log="pipeline.removeLog"
                        @fit-alone="(spec) => pipeline.runFitEach([spec])"
                    />
                    <p v-else-if="store.workspace" class="text-sm text-dimmed">{{ $t("whippyflightNoLogs") }}</p>
                    <p v-if="store.workspace && !store.workspace.hasCliDump" class="text-xs text-dimmed">
                        {{ $t("whippyflightNoDump") }}
                    </p>
                </UiBox>

                <!-- Regression -->
                <UiBox :title="$t('whippyflightStepFit')" :type="boxType('fit')" :highlight="isCurrent('fit')">
                    <div class="flex flex-wrap items-center gap-4 text-sm">
                        <label class="flex items-center gap-2">
                            <span class="text-dimmed">{{ $t("whippyflightDrag") }}</span>
                            <USelect v-model="store.settings.drag" :items="dragItems" size="xs" class="w-28" />
                        </label>
                        <div class="grow"></div>
                        <UButton
                            size="xs"
                            variant="outline"
                            icon="i-lucide-images"
                            :label="$t('whippyflightSavePlots')"
                            :title="plotsHint"
                            :disabled="!canWork || !store.diagnostics || !store.env?.hasMatplotlib"
                            :loading="store.busy === 'plots'"
                            @click="pipeline.savePlots('fit')"
                        />
                        <UButton
                            size="xs"
                            variant="outline"
                            icon="i-lucide-layers"
                            :label="$t('whippyflightRunFitEach')"
                            :disabled="!canWork || store.selectedFlights.length < 2"
                            @click="pipeline.runFitEach(store.selectedFlights)"
                        />
                        <UButton
                            size="xs"
                            icon="i-lucide-chart-scatter"
                            :label="$t('whippyflightRunFit', { count: store.selectedFlights.length })"
                            :disabled="!canWork || !store.selectedFlights.length"
                            :loading="store.busy === 'fit'"
                            @click="pipeline.runFit"
                        />
                    </div>
                    <UProgress
                        v-if="store.busy === 'fit' && store.progress !== null"
                        :model-value="store.progress * 100"
                    />
                    <FitList
                        v-if="store.workspace?.fits.length"
                        :fits="store.workspace.fits"
                        :diagnostics="store.fitDiagnostics"
                        :active="store.activeFit"
                        :gains-fit="store.workspace.gainsFit"
                        :disabled="!canWork"
                        @select="pipeline.selectFit"
                        @remove="pipeline.removeFit"
                    />
                    <DiagnosticsPanel v-if="store.diagnostics" :diagnostics="store.diagnostics" />
                </UiBox>

                <!-- Gains -->
                <UiBox :title="$t('whippyflightStepGains')" :type="boxType('gains')" :highlight="isCurrent('gains')">
                    <div class="flex items-center justify-between gap-4 text-sm">
                        <div v-if="store.gains" class="flex flex-wrap gap-x-5 gap-y-1">
                            <span>
                                <span class="text-dimmed">{{ $t("whippyflightLoopRate") }}</span>
                                <span :class="gainsDtOk ? '' : 'text-error'"> {{ hz(store.gains.info.dt) }}</span>
                            </span>
                            <span :class="gridStale ? 'text-warning' : ''" :title="gridStale">
                                <span class="text-dimmed">{{ $t("whippyflightGrid") }}</span>
                                {{ store.gains.info.resolution }} · {{ store.gains.info.minRps.toFixed(0) }}–{{
                                    store.gains.info.maxRps.toFixed(0)
                                }}
                                rps
                            </span>
                            <span class="font-mono text-xs">
                                v{{ store.gains.info.version }} · {{ store.gains.info.size }} B · CRC
                                {{ store.gains.info.crc.toString(16).padStart(8, "0") }}
                            </span>
                        </div>
                        <span v-else class="text-dimmed">{{ $t("whippyflightNoGainsYet") }}</span>
                        <div class="grow"></div>
                        <UButton
                            v-if="store.gains"
                            size="xs"
                            variant="outline"
                            icon="i-lucide-images"
                            :label="$t('whippyflightSavePlots')"
                            :title="gainPlotsHint"
                            :disabled="!canWork || !store.env?.hasMatplotlib"
                            :loading="store.busy === 'plots'"
                            @click="pipeline.savePlots('gains')"
                        />
                        <label class="flex items-center gap-2" :title="gridHint">
                            <span class="text-dimmed">{{ $t("whippyflightGrid") }}</span>
                            <USelect
                                v-model="store.settings.resolution"
                                :items="gridItems"
                                :disabled="store.busy !== null"
                                size="xs"
                                class="w-36"
                            />
                        </label>
                        <UButton
                            size="xs"
                            variant="outline"
                            icon="i-lucide-chart-scatter"
                            :label="$t('whippyflightExploreFront')"
                            :title="$t('whippyflightExploreFrontHint')"
                            :disabled="!canWork || !store.diagnostics || !pipeline.gainsLoop.value"
                            :loading="store.busy === 'front'"
                            @click="pipeline.exploreFront"
                        />
                        <UButton
                            size="xs"
                            icon="i-lucide-calculator"
                            :label="$t('whippyflightRunGains')"
                            :title="
                                pipeline.gainsLoop.value
                                    ? $t(
                                          pipeline.gainsLoop.value.from === 'log'
                                              ? 'whippyflightRunGainsLog'
                                              : 'whippyflightRunGainsFc',
                                          { rate: hz(pipeline.gainsLoop.value.dt) },
                                      )
                                    : undefined
                            "
                            :disabled="!canWork || !store.diagnostics || !pipeline.gainsLoop.value"
                            :loading="store.busy === 'gen'"
                            @click="pipeline.runGains"
                        />
                    </div>
                    <WeightControls
                        v-model="store.settings.weights"
                        :recorded="store.gains ? (store.workspace?.gainsWeights ?? null) : null"
                        :derived-sigma-rate="activeFront?.derived_sigma_rate ?? null"
                        :derived-tau-deriv="activeFront?.derived_tau_deriv ?? null"
                        :disabled="store.busy !== null"
                    />
                    <FrontPanel
                        v-if="store.front && store.front.fit === store.activeFit"
                        :report="store.front"
                        :current="frontCurrent"
                        :stale="frontStale"
                        :disabled="!canWork"
                        :recompute-disabled="!canWork || !store.diagnostics || !pipeline.gainsLoop.value"
                        @pick="pipeline.pickFrontPoint"
                        @recompute="pipeline.exploreFront"
                    />
                    <p v-if="gridStale" class="text-sm text-warning">{{ gridStale }}</p>
                    <p v-if="store.gains && !gainsFromActiveFit" class="text-sm text-warning">
                        {{
                            store.workspace?.gainsFit
                                ? $t("whippyflightGainsFromOtherFit", {
                                      fit: store.workspace.gainsFit,
                                      interpolation: { escapeValue: false },
                                  })
                                : $t("whippyflightGainsFromUnknownFit")
                        }}
                    </p>
                    <p v-if="store.gains && !gainsDtOk" class="text-sm text-error">
                        {{ $t("whippyflightDtMismatch") }}
                    </p>
                    <div v-if="store.gains" class="flex flex-wrap items-center gap-x-6 gap-y-2">
                        <FlickRateControls
                            v-model="store.settings.flickRates"
                            :report="store.flicks"
                            :disabled="store.busy !== null"
                        />
                        <FlickRiseControls
                            v-model="store.settings.flickRiseMs"
                            :report="store.flicks"
                            :disabled="store.busy !== null"
                        />
                        <div class="flex items-center gap-3 text-xs" :title="$t('whippyflightFlickFeedbackHint')">
                            <span class="text-dimmed">{{ $t("whippyflightFlickFeedback") }}</span>
                            <UCheckbox
                                v-for="term in FEEDBACK_TERMS"
                                :key="term"
                                v-model="store.settings.flickFeedback[term]"
                                size="sm"
                                :label="term.toUpperCase()"
                                :disabled="store.busy !== null"
                            />
                        </div>
                        <div class="grow"></div>
                        <UButton
                            v-if="
                                (!store.flicks ||
                                    flicksRetracked ||
                                    flicksRefiltered ||
                                    flicksRerated ||
                                    flicksResloped ||
                                    flicksRemoded ||
                                    flicksReconditioned) &&
                                store.workspace?.gainsFit
                            "
                            size="xs"
                            variant="outline"
                            icon="i-lucide-activity"
                            :label="
                                flicksRetracked
                                    ? $t('whippyflightRerunFlicks', { lambda: store.stickTracking })
                                    : flicksRefiltered
                                      ? store.motorLpfHz
                                          ? $t('whippyflightRerunFlicksMotorLpf', { hz: store.motorLpfHz })
                                          : $t('whippyflightRerunFlicksMotorLpfOff')
                                      : flicksRerated
                                        ? $t('whippyflightRerunFlicksRates')
                                        : flicksResloped
                                          ? $t('whippyflightRerunFlicksRise')
                                          : flicksRemoded
                                            ? $t('whippyflightRerunFlicksFeedforward')
                                            : flicksReconditioned
                                              ? $t('whippyflightRerunFlicksConditions')
                                              : $t('whippyflightRunFlicks')
                            "
                            :disabled="!canWork"
                            :loading="store.busy === 'flicks'"
                            @click="pipeline.runFlicks"
                        />
                        <FlickConditionsControls
                            v-model="store.settings.flickConditions"
                            :logged-noise="gainsFitRun?.noiseRms ?? null"
                            :logged-turbulence="gainsFitRun?.turbulence ?? null"
                            :logged-latency="gainsFitRun?.latency ?? null"
                            :disabled="store.busy !== null"
                        />
                    </div>
                    <FlickPanel v-if="store.gains && store.flicks" :report="store.flicks" />
                </UiBox>

                <!-- Upload -->
                <UiBox :title="$t('whippyflightStepUpload')" :type="boxType('upload')" :highlight="isCurrent('upload')">
                    <div class="flex items-center justify-between gap-4 text-sm">
                        <span v-if="onFc" class="text-success flex items-center gap-2">
                            <UIcon name="i-lucide-circle-check" class="size-4" />
                            {{ $t("whippyflightGainsOnFc") }}
                        </span>
                        <span v-else-if="store.committing">{{ $t("whippyflightWriting") }}</span>
                        <span v-else class="text-dimmed">{{ $t("whippyflightUploadHint") }}</span>
                        <UButton
                            size="xs"
                            icon="i-lucide-upload"
                            :label="$t('whippyflightUpload')"
                            :disabled="!canWork || !store.gains || !connected || !store.status"
                            :loading="store.busy === 'upload'"
                            @click="pipeline.uploadGains"
                        />
                    </div>
                    <UProgress v-if="store.busy === 'upload'" :model-value="(store.progress ?? 0) * 100" />
                </UiBox>

                <UiBox :title="$t('whippyflightOutput')" type="neutral" collapsible :default-open="false">
                    <pre
                        class="text-xs font-mono max-h-64 overflow-auto whitespace-pre-wrap"
                    ><span v-for="(line, i) in store.output" :key="i" :class="line.stderr ? 'text-dimmed' : ''">{{ line.line }}
</span></pre>
                </UiBox>
            </template>

            <div v-if="store.error || store.busy" class="flex items-center gap-3 text-sm">
                <span v-if="store.error" class="text-error grow">{{ store.error }}</span>
                <span v-else class="text-dimmed grow">{{ $t(`whippyflightBusy_${store.busy}`) }}</span>
                <UButton
                    v-if="['setup', 'scan', 'fit', 'gen', 'flicks', 'erase'].includes(store.busy ?? '')"
                    size="xs"
                    variant="outline"
                    :label="$t('whippyflightCancel')"
                    @click="pipeline.cancel"
                />
            </div>
        </div>
    </BaseTab>
</template>

<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useTranslation } from "i18next-vue";
import BaseTab from "./BaseTab.vue";
import UiBox from "@/components/elements/UiBox.vue";
import GainStatus from "./whippyflight/GainStatus.vue";
import ControllerSettings from "./whippyflight/ControllerSettings.vue";
import DiagnosticsPanel from "./whippyflight/DiagnosticsPanel.vue";
import FitList from "./whippyflight/FitList.vue";
import FlickPanel from "./whippyflight/FlickPanel.vue";
import FlickRateControls from "./whippyflight/FlickRateControls.vue";
import FlickRiseControls from "./whippyflight/FlickRiseControls.vue";
import FlickConditionsControls from "./whippyflight/FlickConditionsControls.vue";
import WeightControls from "./whippyflight/WeightControls.vue";
import FrontPanel from "./whippyflight/FrontPanel.vue";
import { frontIndexOf } from "@/js/whippyflight/front";
import { FEEDBACK_TERMS, flownAtRates, sameConditions, sameFeedback } from "@/js/whippyflight/flicks";
import FlightPicker from "./whippyflight/FlightPicker.vue";
import { GainSource, useWhippyFlightStore } from "@/stores/whippyflight";
import { useConnectionStore } from "@/stores/connection";
import { useWhippyFlightPipeline, type FlowStep } from "@/composables/useWhippyFlightPipeline";
import { useInterval } from "@/composables/useInterval";
import { dtMatches } from "@/js/whippyflight/protocol";
import { GRID_RESOLUTIONS } from "@/js/whippyflight/pipeline";

const STATUS_POLL_MS = 2000;
const FLOW: FlowStep[] = ["setup", "logs", "fit", "gains", "upload"];

const { t } = useTranslation();
const store = useWhippyFlightStore();
const pipeline = useWhippyFlightPipeline();
const { addInterval } = useInterval();

const connectionStore = useConnectionStore();
const connected = computed(() => connectionStore.connectionValid);
const canWork = computed(() => !store.busy && pipeline.currentStep.value !== "setup");
const plotsHint = computed(() =>
    t(store.env?.hasMatplotlib ? "whippyflightSavePlotsHint" : "whippyflightNoMatplotlib"),
);
const gainPlotsHint = computed(() =>
    t(store.env?.hasMatplotlib ? "whippyflightSaveGainPlotsHint" : "whippyflightNoMatplotlib"),
);
const canImport = computed(() => !store.busy && !!store.settings.workdir);

const dragItems = computed(() =>
    (["auto", "on", "off"] as const).map((value) => ({ value, label: t(`whippyflightDrag_${value}`) })),
);

const gainsFromActiveFit = computed(() => store.workspace?.gainsFit === store.activeFit);

const frontCurrent = computed(() =>
    store.front && store.workspace?.gainsFit === store.front.fit
        ? frontIndexOf(store.front, store.workspace.gainsWeights)
        : -1,
);
const activeFront = computed(() => (store.front && store.front.fit === store.activeFit ? store.front : null));
/** Why the front no longer matches the gains "Compute gains" would make, if it doesn't. */
const frontStale = computed(() => {
    const front = store.front;
    const loop = pipeline.gainsLoop.value;
    if (!front) {
        return undefined;
    }
    if (loop && Math.abs(front.dt - loop.dt) > 0.01 * loop.dt) {
        return t("whippyflightFrontStaleLoop", { front: hz(front.dt), now: hz(loop.dt) });
    }
    if (front.sigma_rate !== store.settings.weights.sigmaRate) {
        return t("whippyflightFrontStaleRate", {
            front: front.sigma_rate,
            now: store.settings.weights.sigmaRate,
        });
    }
    if (front.tau_int !== store.settings.weights.tauInt) {
        return t("whippyflightFrontStaleTauInt", { front: front.tau_int, now: store.settings.weights.tauInt });
    }
    if (front.tau_deriv !== store.settings.weights.tauDeriv) {
        return t("whippyflightFrontStaleTauDeriv", {
            front: Number((1e3 * front.tau_deriv).toPrecision(3)),
            now: Number((1e3 * store.settings.weights.tauDeriv).toPrecision(3)),
        });
    }
    const grid = pipeline.gainsGrid.value;
    if (front.resolution == null || front.idle_rps == null) {
        return t("whippyflightFrontStaleGridUnknown");
    }
    if (front.resolution !== grid.resolution || front.idle_rps !== grid.idleRps) {
        return t("whippyflightFrontStaleGrid", {
            front: front.resolution,
            frontIdle: front.idle_rps.toFixed(0),
            now: grid.resolution,
            nowIdle: grid.idleRps.toFixed(0),
        });
    }
    return undefined;
});

const gridItems = GRID_RESOLUTIONS.map((n) => ({
    value: n,
    label: t("whippyflightGridPoints", { n, points: n ** 4 }),
}));

const gridHint = computed(() => {
    const idle = pipeline.gainsGrid.value.idleRps;
    return idle > 0 ? t("whippyflightGridHintIdle", { idle: idle.toFixed(0) }) : t("whippyflightGridHint");
});

/** Why the gains' grid isn't the one "Compute gains" would make now, if it isn't. */
const gridStale = computed(() => {
    const info = store.gains?.info;
    if (!info) {
        return undefined;
    }
    const grid = pipeline.gainsGrid.value;
    if (info.minRps < grid.idleRps - 0.5) {
        return t("whippyflightGridBelowIdle", { min: info.minRps.toFixed(0), idle: grid.idleRps.toFixed(0) });
    }
    if (info.resolution !== grid.resolution) {
        return t("whippyflightGridResolutionDiffers", { gains: info.resolution, now: grid.resolution });
    }
    return undefined;
});

/** Whether the controller settings' stick tracking differs from the one the flicks were flown at. */
const flicksRetracked = computed(
    () => !!store.flicks && store.stickTracking !== null && store.stickTracking !== store.flicks.stick_tracking,
);

/** Whether the controller settings' motor-speed low-pass differs from the one the flicks were flown with. */
const flicksRefiltered = computed(
    () => !!store.flicks && store.motorLpfHz !== null && store.motorLpfHz !== store.flicks.motor_lpf_hz,
);

/** Whether the flick rates set (or unset) by hand differ from those the flicks were flown at. */
const flicksRerated = computed(() => !!store.flicks && !flownAtRates(store.flicks, store.settings.flickRates));

/** Whether the slowest stick set (or unset) differs from the one the flicks were flown with. */
const flicksResloped = computed(
    () => !!store.flicks && (store.flicks.min_rise_ms ?? null) !== store.settings.flickRiseMs,
);

/** Whether the feedback gains picked differ from those the flicks were flown with. */
const flicksRemoded = computed(
    () => !!store.flicks && !sameFeedback(store.flicks.feedback, store.settings.flickFeedback),
);

/** Whether the flick conditions differ from those the flicks were flown in. */
const gainsFitRun = computed(() => store.workspace?.fits.find((f) => f.id === store.workspace?.gainsFit));
const flicksReconditioned = computed(
    () => !!store.flicks && !sameConditions(store.flicks.conditions, store.settings.flickConditions),
);

const gainsDtOk = computed(
    () => !store.gains || !connected.value || !store.status || dtMatches(store.gains.info.dt, store.status.pidDt),
);

const onFc = computed(
    () =>
        !!store.gains &&
        !!store.status &&
        store.status.source === GainSource.DATAFLASH &&
        store.status.blobCrc === store.gains.info.crc &&
        store.status.blobSize === store.gains.info.size,
);

function isCurrent(step: FlowStep): boolean {
    return pipeline.currentStep.value === step;
}

function boxType(step: FlowStep): string {
    if (step === "upload" && onFc.value) {
        return "success";
    }
    return FLOW.indexOf(step) < FLOW.indexOf(pipeline.currentStep.value) ? "success" : "default";
}

function hz(dt: number): string {
    return dt > 0 ? `${Math.round(1 / dt)} Hz` : "–";
}

function megabytes(bytes: number): string {
    return `${(bytes / (1024 * 1024)).toFixed(bytes < 1024 * 1024 ? 2 : 1)} MiB`;
}

async function checkEnv(): Promise<void> {
    await pipeline.checkEnvironment();
    await pipeline.scanLogs();
}

async function refreshAll(): Promise<void> {
    if (connected.value) {
        await store.refreshStatus().catch(() => undefined);
        await pipeline.refreshFlash();
    }
    await pipeline.refreshWorkspace();
}

onMounted(async () => {
    await refreshAll();
    await checkEnv();
    addInterval(
        "whippyflight_status",
        () => {
            if (!store.busy && connected.value) {
                void store.refreshStatus().catch(() => undefined);
            }
        },
        STATUS_POLL_MS,
    );
});
</script>
