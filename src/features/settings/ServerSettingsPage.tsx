import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useForm } from "react-hook-form";
import { useTranslation } from "react-i18next";
import { z } from "zod";
import { HelpTip } from "../../components/HelpTip";
import {
  getFrpcStatus,
  getServerSettings,
  recoverServerToken,
  runFrpcAction,
  saveServerSettings,
  testFrpcConnectivity,
} from "../../lib/api";
import type { ServerSettings } from "../../lib/types";
import { FrpsSetupGuide } from "./FrpsSetupGuide";

const DEFAULT_FRPC_IMAGE = "ghcr.io/fatedier/frpc:v0.70.1@sha256:e6483f2a916de67281597ba8fd03dc25d4f6fbd7ed0eafa042b2a5e4dcb5ee22";

const schema = z.object({
  frpsHost: z.string().trim().min(1).max(253),
  frpsPort: z.coerce.number().int().min(1).max(65535),
  publicIp: z.string().trim().min(1).max(253),
  remotePortStart: z.coerce.number().int().min(1).max(65535),
  remotePortEnd: z.coerce.number().int().min(1).max(65535),
  frpcImage: z.string().trim().min(1).refine((value) => !/\s/.test(value)),
}).refine((value) => value.remotePortStart <= value.remotePortEnd, {
  path: ["remotePortEnd"],
  message: "invalid range",
});

type FormValues = z.infer<typeof schema>;

const defaults: FormValues = {
  frpsHost: "",
  frpsPort: 7000,
  publicIp: "",
  remotePortStart: 41000,
  remotePortEnd: 42000,
  frpcImage: DEFAULT_FRPC_IMAGE,
};

export function ServerSettingsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [showGuide, setShowGuide] = useState(false);
  const settings = useQuery({ queryKey: ["server-settings"], queryFn: getServerSettings });
  const frpcStatus = useQuery({ queryKey: ["frpc-status"], queryFn: getFrpcStatus, refetchInterval: 3000 });
  const { register, handleSubmit, reset, setValue, formState: { errors, isDirty } } = useForm<FormValues>({
    resolver: zodResolver(schema),
    defaultValues: defaults,
  });
  useEffect(() => {
    if (!settings.data) return;
    reset(settings.data);
  }, [reset, settings.data]);

  const save = useMutation({
    mutationFn: (next: ServerSettings) => saveServerSettings(next),
    onSuccess: (saved) => {
      queryClient.setQueryData(["server-settings"], saved);
      queryClient.removeQueries({ queryKey: ["frps-setup-guide"] });
      reset(saved);
    },
  });

  const serviceAction = useMutation({
    mutationFn: runFrpcAction,
    onSuccess: (status) => queryClient.setQueryData(["frpc-status"], status),
  });

  const connectivity = useMutation({
    mutationFn: testFrpcConnectivity,
    onSuccess: (result) => {
      if (result.success) void queryClient.invalidateQueries({ queryKey: ["server-settings"] });
    },
  });

  const recovery = useMutation({
    mutationFn: recoverServerToken,
    onSuccess: (saved) => {
      queryClient.setQueryData(["server-settings"], saved);
      queryClient.removeQueries({ queryKey: ["frps-setup-guide"] });
      void queryClient.invalidateQueries({ queryKey: ["frpc-status"] });
      setShowGuide(true);
    },
  });

  const submit = (values: FormValues) => {
    const next: ServerSettings = {
      frpsHost: values.frpsHost,
      frpsPort: values.frpsPort,
      publicIp: values.publicIp,
      remotePortStart: values.remotePortStart,
      remotePortEnd: values.remotePortEnd,
      tokenConfigured: settings.data?.tokenConfigured ?? false,
      tokenState: settings.data?.tokenState ?? "missing",
      frpcImage: values.frpcImage,
    };
    save.mutate(next);
  };

  const configured = Boolean(settings.data?.frpsHost && settings.data.publicIp);
  const tokenState = settings.data?.tokenState;
  const tokenReady = tokenState === "ready";
  const tokenAvailable = tokenReady || tokenState === "recovery_pending";
  const actionsDisabled = isDirty || !configured || serviceAction.isPending || connectivity.isPending;

  if (showGuide) return <FrpsSetupGuide onClose={() => setShowGuide(false)} />;

  return (
    <section aria-labelledby="settings-title">
      <header className="page-header">
        <div><p className="eyebrow">FRP SERVER</p><h2 id="settings-title">{t("settings.title")}</h2><p>{t("settings.description")}</p></div>
      </header>
      {tokenState === "missing" ? (
        <div className="security-banner warning-banner settings-width" role="alert">
          <span aria-hidden="true">!</span>
          <div><strong>{t("settings.recovery.missingTitle")}</strong><p>{t("settings.recovery.missingHint")}</p></div>
          <button type="button" className="secondary-button" disabled={isDirty || recovery.isPending} onClick={() => recovery.mutate()}>{t(recovery.isPending ? "settings.recovery.starting" : "settings.recovery.start")}</button>
        </div>
      ) : tokenState === "recovery_pending" ? (
        <div className="security-banner warning-banner settings-width" role="status">
          <span aria-hidden="true">!</span>
          <div><strong>{t("settings.recovery.pendingTitle")}</strong><p>{t("settings.recovery.pendingHint")}</p></div>
          <button type="button" className="secondary-button" disabled={isDirty} onClick={() => setShowGuide(true)}>{t("settings.recovery.openGuide")}</button>
        </div>
      ) : (
        <div className="security-banner settings-width"><span aria-hidden="true">◇</span><div><strong>{t("settings.secretTitle")}</strong><p>{t("settings.secretHint")}</p></div></div>
      )}
      {recovery.isError ? <div className="inline-error" role="alert">{t("settings.recovery.failed")}</div> : null}
      {settings.isLoading ? <p className="muted">{t("settings.loading")}</p> : null}
      {settings.isError ? <div className="inline-error" role="alert">{t("settings.loadFailed")}</div> : null}
      {!settings.isLoading && !settings.isError ? (
        <form className="settings-panel" onSubmit={handleSubmit(submit)}>
          <div className="form-grid">
            <label className="field full"><span className="field-label">{t("settings.frpsHost")}<HelpTip label={t("settings.frpsHost")} text={t("settings.help.frpsHost")} /></span><input {...register("frpsHost")} placeholder="frps.example.com" aria-invalid={!!errors.frpsHost} /></label>
            <label className="field"><span className="field-label">{t("settings.frpsPort")}<HelpTip label={t("settings.frpsPort")} text={t("settings.help.frpsPort")} /></span><input type="number" {...register("frpsPort")} aria-invalid={!!errors.frpsPort} /></label>
            <label className="field"><span className="field-label">{t("settings.publicIp")}<HelpTip label={t("settings.publicIp")} text={t("settings.help.publicIp")} /></span><input {...register("publicIp")} placeholder="203.0.113.10" aria-invalid={!!errors.publicIp} /></label>
            <div className="form-section full"><h3>{t("settings.portRange")}</h3></div>
            <label className="field"><span className="field-label">{t("settings.portStart")}<HelpTip label={t("settings.portStart")} text={t("settings.help.portStart")} /></span><input type="number" {...register("remotePortStart")} aria-invalid={!!errors.remotePortStart} /></label>
            <label className="field"><span className="field-label">{t("settings.portEnd")}<HelpTip label={t("settings.portEnd")} text={t("settings.help.portEnd")} /></span><input type="number" {...register("remotePortEnd")} aria-invalid={!!errors.remotePortEnd} />{errors.remotePortEnd ? <small className="field-error">{t("settings.invalidRange")}</small> : null}</label>
            <div className="form-section full"><h3>{t("settings.client")}</h3></div>
            <div className="field full"><label className="field-label" htmlFor="frpc-image">{t("settings.frpcImage")}<HelpTip label={t("settings.frpcImage")} text={t("settings.help.frpcImage")} /></label><div className="field-with-action"><input id="frpc-image" {...register("frpcImage")} spellCheck={false} aria-invalid={!!errors.frpcImage} /><button type="button" className="secondary-button" onClick={() => setValue("frpcImage", DEFAULT_FRPC_IMAGE, { shouldDirty: true, shouldValidate: true })}>{t("settings.restoreDefault")}</button></div></div>
          </div>
          <footer className="dialog-actions">
            <button type="button" className="secondary-button" disabled={isDirty || !configured || tokenState === "missing"} onClick={() => setShowGuide(true)}>{t("settings.openGuide")}</button>
            {save.isSuccess ? <span className="save-success" role="status">{t("settings.saved")}</span> : null}
            {save.isError ? <span className="field-error" role="alert">{t("settings.saveFailed")}</span> : null}
            <button type="submit" className="primary-button" disabled={save.isPending}>{t(save.isPending ? "settings.saving" : "settings.save")}</button>
          </footer>
        </form>
      ) : null}
      {!settings.isLoading && !settings.isError ? (
        <section className="settings-panel service-panel" aria-labelledby="frpc-service-title">
          <div className="service-heading"><div><p className="eyebrow">FRPC</p><h3 id="frpc-service-title">{t("settings.service.title")}</h3></div><span className={frpcStatus.data?.connected ? "status-badge installed" : "status-badge"}>{t(`settings.service.states.${frpcStatus.data?.state ?? "not_created"}`)}</span></div>
          <p className="muted">{t(frpcStatus.data?.connected ? "settings.service.connected" : "settings.service.notConnected")}</p>
          {frpcStatus.data?.image ? <code className="service-image">{frpcStatus.data.image}</code> : null}
          {isDirty ? <p className="field-error">{t("settings.service.saveFirst")}</p> : null}
          <div className="service-actions">
            <button type="button" className="primary-button" disabled={actionsDisabled || !tokenReady} onClick={() => serviceAction.mutate("start")}>{t("common.start")}</button>
            <button type="button" className="secondary-button" disabled={actionsDisabled || !tokenReady || frpcStatus.data?.state !== "running"} onClick={() => serviceAction.mutate("restart")}>{t("common.restart")}</button>
            <button type="button" className="secondary-button" disabled={actionsDisabled || frpcStatus.data?.state !== "running"} onClick={() => serviceAction.mutate("stop")}>{t("common.stop")}</button>
            <button type="button" className="secondary-button" disabled={actionsDisabled || !tokenAvailable} onClick={() => connectivity.mutate()}>{t(connectivity.isPending ? "settings.service.testing" : "settings.service.test")}</button>
          </div>
          {serviceAction.isError ? <div className="inline-error" role="alert">{t("settings.service.actionFailed")}</div> : null}
          {connectivity.data ? <p className={connectivity.data.success ? "save-success" : "field-error"} role="status">{t(`settings.service.testResults.${connectivity.data.code}`)}</p> : null}
          {connectivity.isError ? <div className="inline-error" role="alert">{t("settings.service.testFailed")}</div> : null}
        </section>
      ) : null}
    </section>
  );
}
