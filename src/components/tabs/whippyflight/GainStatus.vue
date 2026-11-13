<template>
    <div class="grid grid-cols-2 md:grid-cols-4 gap-3 text-sm">
        <div class="flex flex-col">
            <span class="text-xs text-dimmed">{{ $t("whippyflightGains") }}</span>
            <span :class="status.gainResult === GainLoadResult.OK ? 'text-success font-bold' : 'text-error font-bold'">
                {{ $t(`whippyflightGain_${GainLoadResult[status.gainResult]}`) }}
            </span>
        </div>
        <div class="flex flex-col">
            <span class="text-xs text-dimmed">{{ $t("whippyflightSource") }}</span>
            <span>{{ $t(`whippyflightSource_${GainSource[status.source]}`) }}</span>
        </div>
        <div class="flex flex-col">
            <span class="text-xs text-dimmed">{{ $t("whippyflightLoopRate") }}</span>
            <span :class="loaded && !dtOk ? 'text-error' : ''">
                {{ hz(status.pidDt) }}
                <template v-if="loaded">({{ $t("whippyflightGainsFor", { rate: hz(status.blobDt) }) }})</template>
            </span>
        </div>
        <div class="flex flex-col">
            <span class="text-xs text-dimmed">{{ $t("whippyflightArming") }}</span>
            <span v-if="!status.active" class="text-dimmed">{{ $t("whippyflightArmingInactive") }}</span>
            <span v-else :class="status.armingBlockReason ? 'text-warning' : 'text-success'">
                {{ status.armingBlockReason || $t("whippyflightArmingReady") }}
            </span>
        </div>
        <div v-if="loaded" class="flex flex-col">
            <span class="text-xs text-dimmed">{{ $t("whippyflightBlob") }}</span>
            <span class="font-mono text-xs">{{ status.blobSize }} B · CRC {{ crcHex(status.blobCrc) }}</span>
        </div>
        <div class="flex flex-col">
            <span class="text-xs text-dimmed">{{ $t("whippyflightGainStore") }}</span>
            <span>{{
                status.partitionSize ? `${(status.partitionSize / 1024).toFixed(0)} KiB` : $t("whippyflightNone")
            }}</span>
        </div>
    </div>
    <p v-if="status.source === GainSource.SDCARD" class="text-sm text-warning flex gap-2 items-start">
        <UIcon name="i-lucide-triangle-alert" class="size-4 shrink-0 mt-0.5" />
        {{ $t("whippyflightSdCardWins") }}
    </p>
</template>

<script setup lang="ts">
import { computed } from "vue";
import { GainLoadResult, GainSource } from "@/stores/whippyflight";
import { dtMatches, type WhippyFlightStatus } from "@/js/whippyflight/protocol";

const props = defineProps<{ status: WhippyFlightStatus }>();

const loaded = computed(() => props.status.gainResult === GainLoadResult.OK);
const dtOk = computed(() => dtMatches(props.status.blobDt, props.status.pidDt));

function hz(dt: number): string {
    return dt > 0 ? `${Math.round(1 / dt)} Hz` : "–";
}

function crcHex(crc: number): string {
    return crc.toString(16).padStart(8, "0");
}
</script>
