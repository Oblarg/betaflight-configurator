<template>
    <div class="flex flex-col gap-2 text-sm">
        <div v-if="usable.length" class="flex flex-wrap items-center gap-2">
            <span class="text-dimmed grow">
                {{ $t("whippyflightSelectedFlights", { selected: selected.length, usable: usable.length }) }}
            </span>
            <UButton
                size="xs"
                variant="outline"
                :label="$t('whippyflightSelectAll')"
                :disabled="disabled"
                @click="selected = usable.map((f) => f.spec)"
            />
            <UButton
                size="xs"
                variant="outline"
                :label="$t('whippyflightSelectLatest')"
                :disabled="disabled"
                @click="selected = [usable[usable.length - 1].spec]"
            />
            <UButton
                size="xs"
                variant="outline"
                :label="$t('whippyflightSelectNone')"
                :disabled="disabled"
                @click="selected = []"
            />
        </div>

        <div v-for="log in logs" :key="log.name" class="flex flex-col">
            <div class="flex items-center gap-2 border-b border-default py-1">
                <UIcon name="i-lucide-file" class="size-4 text-dimmed" />
                <span class="font-mono text-xs grow">{{ log.name }}</span>
                <span class="text-xs text-dimmed">{{ megabytes(log.size) }}</span>
                <UButton
                    size="xs"
                    variant="ghost"
                    color="neutral"
                    icon="i-lucide-x"
                    :disabled="disabled"
                    :aria-label="$t('whippyflightDeleteLog')"
                    @click="emit('remove-log', log.name)"
                />
            </div>
            <p v-if="unscanned.includes(log.name)" class="text-xs text-dimmed py-1 pl-6">
                {{ $t("whippyflightNotScanned") }}
            </p>
            <p v-else-if="index?.[log.name]?.error" class="text-xs text-error py-1 pl-6">
                {{
                    $t("whippyflightScanFailed", {
                        error: index[log.name].error,
                        interpolation: { escapeValue: false },
                    })
                }}
            </p>
            <table v-else class="text-xs w-full">
                <tbody>
                    <tr
                        v-for="flight in byLog[log.name]?.shown ?? []"
                        :key="flight.spec"
                        :class="flight.usable ? '' : 'text-dimmed'"
                    >
                        <td class="w-6 pl-1 py-0.5">
                            <input
                                v-model="selected"
                                type="checkbox"
                                class="size-4 align-middle"
                                :value="flight.spec"
                                :disabled="disabled || !flight.usable"
                                :aria-label="flight.spec"
                            />
                        </td>
                        <td class="w-8 font-mono">#{{ flight.entry.index }}</td>
                        <td class="w-36">{{ startTime(flight.entry.start) }}</td>
                        <td class="w-28">
                            {{
                                $t("whippyflightAirborne", {
                                    kept: flight.entry.kept_s ?? "–",
                                    duration: flight.entry.duration_s,
                                })
                            }}
                        </td>
                        <td v-if="flight.usable">
                            <span class="text-dimmed">{{ $t("whippyflightExcited") }}</span>
                            <span v-for="axis in EXCITED_AXES" :key="axis" class="ml-2">
                                {{ $t(`whippyflightAxis_${axis}`) }} {{ flight.entry.excited_s?.[axis] ?? "–" }} s
                            </span>
                            <span v-if="flight.entry.gating?.impacts" class="ml-2 text-warning">
                                {{ $t("whippyflightImpacts", { count: flight.entry.gating.impacts }) }}
                            </span>
                        </td>
                        <td v-else>{{ flight.entry.reason }}</td>
                        <td class="w-8 text-right">
                            <UButton
                                v-if="flight.usable"
                                size="xs"
                                variant="ghost"
                                icon="i-lucide-chart-scatter"
                                :disabled="disabled"
                                :title="$t('whippyflightFitAlone')"
                                :aria-label="$t('whippyflightFitAlone')"
                                @click="emit('fit-alone', flight.spec)"
                            />
                        </td>
                    </tr>
                </tbody>
            </table>
            <p v-if="byLog[log.name]?.short" class="text-xs text-dimmed pl-6">
                {{ $t("whippyflightShortLogsHidden", { count: byLog[log.name].short }) }}
            </p>
        </div>
    </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import type { FlightIndex, LogFile } from "@/js/whippyflight/pipeline";
import type { Flight } from "@/js/whippyflight/flights";

const EXCITED_AXES = ["yaw", "pitch", "roll"] as const;

const props = defineProps<{
    logs: LogFile[];
    flights: Flight[];
    index: FlightIndex | null;
    unscanned: string[];
    disabled: boolean;
}>();
const emit = defineEmits<{ "remove-log": [name: string]; "fit-alone": [spec: string] }>();
const selected = defineModel<string[]>({ required: true });

const usable = computed(() => props.flights.filter((flight) => flight.usable));

/** Per log: the flights worth listing, and how many too-short ones (arming blips) are left out. */
const byLog = computed(() => {
    const out: Record<string, { shown: Flight[]; short: number }> = {};
    for (const flight of props.flights) {
        const group = (out[flight.log] ??= { shown: [], short: 0 });
        if (flight.entry.kept_s === undefined) {
            group.short++;
        } else {
            group.shown.push(flight);
        }
    }
    return out;
});

function startTime(start?: string): string {
    if (!start) {
        return "–";
    }
    const date = new Date(start);
    return Number.isNaN(date.getTime())
        ? start
        : date.toLocaleString(undefined, { dateStyle: "short", timeStyle: "medium" });
}

function megabytes(bytes: number): string {
    return `${(bytes / (1024 * 1024)).toFixed(bytes < 1024 * 1024 ? 2 : 1)} MiB`;
}
</script>
