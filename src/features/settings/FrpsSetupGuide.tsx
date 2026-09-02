import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useForm } from "react-hook-form";
import { useTranslation } from "react-i18next";
import { z } from "zod";
import {
  getFrpsSetupGuide,
  getServerSettings,
  saveServerSettings,
} from "../../lib/api";
import type { FrpsSetupGuide as FrpsSetupGuideData, ServerSettings } from "../../lib/types";

interface FrpsSetupGuideProps {
  onClose: () => void;
}

type GuideScenario = "docker" | "native";

const parameterSchema = z
  .object({
    bindPort: z.coerce.number().int().min(1).max(65535),
    remotePortStart: z.coerce.number().int().min(1).max(65535),
    remotePortEnd: z.coerce.number().int().min(1).max(65535),
  })
  .refine((value) => value.remotePortStart <= value.remotePortEnd, {
    path: ["remotePortEnd"],
    message: "invalid range",
  });

type GuideParameters = z.infer<typeof parameterSchema>;

interface GeneratedGuide {
  guide: FrpsSetupGuideData;
  settings: ServerSettings;
}

export function FrpsSetupGuide({ onClose }: FrpsSetupGuideProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [scenario, setScenario] = useState<GuideScenario>("docker");
  const [copiedBlock, setCopiedBlock] = useState<string | null>(null);
  const [copyFailed, setCopyFailed] = useState(false);
  const settings = useQuery({
    queryKey: ["server-settings"],
    queryFn: getServerSettings,
  });
  const {
    register,
    handleSubmit,
    reset,
    formState: { errors, isDirty },
  } = useForm<GuideParameters>({
    resolver: zodResolver(parameterSchema),
    defaultValues: {
      bindPort: 7000,
      remotePortStart: 41000,
      remotePortEnd: 42000,
    },
  });

  useEffect(() => {
    if (!settings.data || isDirty) return;
    reset({
      bindPort: settings.data.frpsPort,
      remotePortStart: settings.data.remotePortStart,
      remotePortEnd: settings.data.remotePortEnd,
    });
  }, [isDirty, reset, settings.data]);

  const generation = useMutation<GeneratedGuide, Error, GuideParameters>({
    mutationFn: async (parameters) => {
      if (!settings.data) throw new Error("server settings unavailable");
      const saved = await saveServerSettings({
        ...settings.data,
        frpsPort: parameters.bindPort,
        remotePortStart: parameters.remotePortStart,
        remotePortEnd: parameters.remotePortEnd,
      });
      return { settings: saved, guide: await getFrpsSetupGuide() };
    },
    onSuccess: ({ settings: saved }, parameters) => {
      queryClient.setQueryData(["server-settings"], saved);
      reset(parameters);
      setCopiedBlock(null);
      setCopyFailed(false);
    },
  });

  const copyContent = async (block: string, content: string) => {
    try {
      await navigator.clipboard.writeText(content);
      setCopiedBlock(block);
      setCopyFailed(false);
    } catch {
      setCopyFailed(true);
    }
  };

  const guide = isDirty ? null : generation.data?.guide;

  return (
    <section aria-labelledby="frps-guide-title">
      <header className="page-header">
        <div>
          <p className="eyebrow">FRPS</p>
          <h2 id="frps-guide-title">{t("settings.guide.title")}</h2>
          <p>{t("settings.guide.description")}</p>
        </div>
        <button type="button" className="secondary-button" onClick={onClose}>
          {t("settings.guide.back")}
        </button>
      </header>

      <div className="security-banner warning-banner">
        <span aria-hidden="true">!</span>
        <div>
          <strong>{t("settings.guide.secretTitle")}</strong>
          <p>{t("settings.guide.secretHint")}</p>
        </div>
      </div>

      {settings.isLoading ? <p className="muted">{t("settings.loading")}</p> : null}
      {settings.isError ? <div className="inline-error" role="alert">{t("settings.loadFailed")}</div> : null}
      {settings.data ? (
        <div className="settings-panel guide-panel">
          <form
            className="guide-generator"
            onSubmit={handleSubmit((parameters) => generation.mutate(parameters))}
          >
            <div className="guide-generator-heading">
              <div>
                <h3>{t("settings.guide.parameterTitle")}</h3>
                <p>{t("settings.guide.parameterHint")}</p>
              </div>
              <button type="submit" className="primary-button" disabled={generation.isPending}>
                {t(generation.isPending ? "settings.guide.generating" : "settings.guide.generate")}
              </button>
            </div>
            <div className="guide-parameter-grid">
              <label className="field">
                <span className="field-label">{t("settings.guide.bindPort")}</span>
                <input
                  type="number"
                  disabled={generation.isPending}
                  {...register("bindPort")}
                  aria-invalid={!!errors.bindPort}
                />
                {errors.bindPort ? <small className="field-error">{t("settings.guide.invalidPort")}</small> : null}
              </label>
              <label className="field">
                <span className="field-label">{t("settings.portStart")}</span>
                <input
                  type="number"
                  disabled={generation.isPending}
                  {...register("remotePortStart")}
                  aria-invalid={!!errors.remotePortStart}
                />
                {errors.remotePortStart ? <small className="field-error">{t("settings.guide.invalidPort")}</small> : null}
              </label>
              <label className="field">
                <span className="field-label">{t("settings.portEnd")}</span>
                <input
                  type="number"
                  disabled={generation.isPending}
                  {...register("remotePortEnd")}
                  aria-invalid={!!errors.remotePortEnd}
                />
                {errors.remotePortEnd ? (
                  <small className="field-error">
                    {t(
                      errors.remotePortEnd.message === "invalid range"
                        ? "settings.invalidRange"
                        : "settings.guide.invalidPort",
                    )}
                  </small>
                ) : null}
              </label>
            </div>
          </form>

          {generation.isError ? (
            <div className="inline-error" role="alert">
              {t("settings.guide.generateFailed")}
            </div>
          ) : null}
          {generation.data && isDirty ? (
            <p className="guide-stale" role="status">{t("settings.guide.stale")}</p>
          ) : null}
          {!generation.data ? <p className="guide-placeholder">{t("settings.guide.notGenerated")}</p> : null}

          {guide ? (
            <div className="generated-guide">
              <div className="guide-isolation-note" role="note">
                <strong>{t("settings.guide.isolationTitle")}</strong>
                <p>
                  {t("settings.guide.isolationHint", {
                    bind: guide.bindPort,
                    start: guide.remotePortStart,
                    end: guide.remotePortEnd,
                  })}
                </p>
              </div>
              <div className="guide-scenarios" role="tablist" aria-label={t("settings.guide.scenarioLabel")}>
                {(["docker", "native"] as const).map((item) => (
                  <button
                    key={item}
                    id={`guide-tab-${item}`}
                    type="button"
                    role="tab"
                    aria-selected={scenario === item}
                    aria-controls={`guide-panel-${item}`}
                    className={scenario === item ? "guide-scenario selected" : "guide-scenario"}
                    onClick={() => {
                      setScenario(item);
                      setCopiedBlock(null);
                      setCopyFailed(false);
                    }}
                  >
                    <strong>{t(`settings.guide.scenarios.${item}.title`)}</strong>
                    <span>{t(`settings.guide.scenarios.${item}.description`)}</span>
                  </button>
                ))}
              </div>

              <div id={`guide-panel-${scenario}`} role="tabpanel" aria-labelledby={`guide-tab-${scenario}`}>
                <ol className="guide-steps">
                  <li>{t(`settings.guide.${scenario}.stepPrerequisite`)}</li>
                  <li>{t(`settings.guide.${scenario}.stepRun`)}</li>
                  <li>
                    {t("settings.guide.stepFirewall", {
                      start: guide.remotePortStart,
                      end: guide.remotePortEnd,
                      bind: guide.bindPort,
                    })}
                  </li>
                  <li>{t("settings.guide.stepReturn")}</li>
                </ol>
                <div className="code-header">
                  <strong>{t(`settings.guide.${scenario}.command`)}</strong>
                  <button
                    type="button"
                    className="secondary-button small-button"
                    onClick={() =>
                      void copyContent(
                        scenario,
                        scenario === "docker"
                          ? guide.dockerSetupScript
                          : guide.nativeSetupScript,
                      )}
                  >
                    {t(copiedBlock === scenario ? "settings.guide.copied" : "settings.guide.copy")}
                  </button>
                </div>
                <pre className="command-block">
                  <code>
                    {scenario === "docker"
                      ? guide.dockerSetupScript
                      : guide.nativeSetupScript}
                  </code>
                </pre>
              </div>
              {copyFailed ? <p className="field-error" role="alert">{t("settings.guide.copyFailed")}</p> : null}
              <p className="muted">{t("settings.guide.cloudFirewall")}</p>
              <p className="muted">{t("settings.guide.publicAddress", { address: guide.publicAddress })}</p>
            </div>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
