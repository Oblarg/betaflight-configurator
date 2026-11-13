<template>
    <div class="flex flex-col gap-1">
        <div class="flex items-center gap-2 text-sm">
            <span class="text-dimmed grow">{{ $t("whippyflightFits") }}</span>
            <UButton
                size="xs"
                variant="outline"
                icon="i-lucide-chevron-left"
                :disabled="position <= 0"
                :aria-label="$t('whippyflightPreviousFit')"
                @click="step(-1)"
            />
            <span class="text-xs tabular-nums">{{ position + 1 }} / {{ fits.length }}</span>
            <UButton
                size="xs"
                variant="outline"
                icon="i-lucide-chevron-right"
                :disabled="position >= fits.length - 1"
                :aria-label="$t('whippyflightNextFit')"
                @click="step(1)"
            />
        </div>
        <table class="text-xs w-full">
            <thead>
                <tr class="text-dimmed text-left">
                    <th class="font-normal py-1">{{ $t("whippyflightFitFlights") }}</th>
                    <th v-for="axis in DIAGNOSTICS_AXES" :key="axis" class="font-normal">
                        {{ $t(`whippyflightAxis_${axis}`) }}
                    </th>
                    <th class="font-normal">{{ $t("whippyflightFitUsed") }}</th>
                    <th></th>
                </tr>
            </thead>
            <tbody>
                <tr
                    v-for="fit in fits"
                    :key="fit.id"
                    class="cursor-pointer hover:bg-elevated"
                    :class="{ 'bg-elevated font-semibold': fit.id === active }"
                    @click="emit('select', fit.id)"
                >
                    <td class="py-1 max-w-80 truncate" :title="describe(fit.id).full">
                        {{ describe(fit.id).short }}
                        <span v-if="dragOf(fit.id)" class="text-dimmed">· {{ dragOf(fit.id) }}</span>
                        <UBadge
                            v-if="fit.id === gainsFit"
                            size="sm"
                            variant="subtle"
                            class="ml-1"
                            :label="$t('whippyflightFitHasGains')"
                        />
                    </td>
                    <template v-if="diagnostics[fit.id]">
                        <td v-for="axis in DIAGNOSTICS_AXES" :key="axis" class="tabular-nums">
                            {{ diagnostics[fit.id].diagnostics.axes[axis].heldout_r2?.toFixed(3) ?? "–" }}
                        </td>
                        <td class="tabular-nums">
                            {{ diagnostics[fit.id].diagnostics.kept_s }} s
                            <span v-if="diagnostics[fit.id].diagnostics.warnings.length" class="text-warning ml-1">
                                <UIcon name="i-lucide-triangle-alert" class="size-3 align-[-1px]" />
                                {{ diagnostics[fit.id].diagnostics.warnings.length }}
                            </span>
                        </td>
                    </template>
                    <td v-else :colspan="DIAGNOSTICS_AXES.length + 1" class="text-error">
                        {{ $t("whippyflightFitFailed") }}
                    </td>
                    <td class="text-right whitespace-nowrap">
                        <UButton
                            size="xs"
                            variant="ghost"
                            color="neutral"
                            icon="i-lucide-x"
                            :disabled="disabled"
                            :aria-label="$t('whippyflightDeleteFit')"
                            @click.stop="emit('remove', fit.id)"
                        />
                    </td>
                </tr>
            </tbody>
        </table>
        <p class="text-xs text-dimmed">
            {{ $t("whippyflightFitColumns", { method: activeMethod, interpolation: { escapeValue: false } }) }}
        </p>
    </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import { useTranslation } from "i18next-vue";
import { DIAGNOSTICS_AXES, type Diagnostics } from "@/js/whippyflight/diagnostics";
import type { FitRun } from "@/js/whippyflight/pipeline";
import { specOfLabel } from "@/js/whippyflight/flights";

/** Flights listed by name before the rest are counted. */
const MAX_NAMED = 2;

const props = defineProps<{
    fits: FitRun[];
    diagnostics: Record<string, { diagnostics: Diagnostics }>;
    active: string | null;
    gainsFit: string | null;
    disabled: boolean;
}>();
const emit = defineEmits<{ select: [id: string]; remove: [id: string] }>();

const { t } = useTranslation();

const position = computed(() => props.fits.findIndex((fit) => fit.id === props.active));
const activeMethod = computed(
    () => (props.active && props.diagnostics[props.active]?.diagnostics.heldout_method) || "–",
);

function step(delta: number): void {
    const next = props.fits[position.value + delta];
    if (next) {
        emit("select", next.id);
    }
}

function describe(id: string): { short: string; full: string } {
    const flights = props.diagnostics[id]?.diagnostics.flights.map((f) => specOfLabel(f.label));
    if (!flights?.length) {
        return { short: id, full: id };
    }
    const named = flights.slice(0, MAX_NAMED).join(", ");
    const rest = flights.length - MAX_NAMED;
    return {
        short:
            rest > 0
                ? t("whippyflightFitAndMore", { named, count: rest, interpolation: { escapeValue: false } })
                : named,
        full: flights.join("\n"),
    };
}

function dragOf(id: string): string {
    const mode = /_drag-(on|off)$/.exec(id)?.[1];
    return mode ? t("whippyflightDragForced", { mode: t(`whippyflightDrag_${mode}`) }) : "";
}
</script>
