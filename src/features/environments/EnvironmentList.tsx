import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  createEnvironment,
  createEnvironmentFromTemplate,
  getEnvironmentCredentials,
  getFrpcStatus,
  getServerSettings,
  getTemplatePreflight,
  listEnvironments,
  listOfficialImages,
  listTemplates,
  openEnvironmentDataDirectory,
  openLocalEnvironment,
  openPublicEnvironment,
  pruneImageCache,
  removeEnvironment,
  runEnvironmentAction,
  runEnvironmentPublicationAction,
  createTemplate,
  getOperation,
} from "../../lib/api";
import type { EnvironmentRecord, EnvironmentSpec, PersistentOperation, TemplatePreflight } from "../../lib/types";
import { forgetOperation, trackOperation } from "../../lib/persistentOperations";
import { CreateEnvironmentDialog } from "./CreateEnvironmentDialog";
import { DeleteEnvironmentDialog } from "./DeleteEnvironmentDialog";
import { useOfficialImagePull } from "../images/useOfficialImagePull";
import { ImageCachePruneDialog } from "../images/ImageCachePruneDialog";
import { useOperationFeedback, type OperationKind } from "../../components/OperationFeedbackContext";
import { SaveTemplateDialog } from "../templates/SaveTemplateDialog";
import { OperationConsole } from "../../components/OperationConsole";

const terminalOperationPhases = new Set(["succeeded", "failed", "cancelled", "retryable"]);

async function waitForOperation(initial: PersistentOperation, onUpdate: (operation: PersistentOperation) => void): Promise<PersistentOperation> {
  let operation = initial;
  onUpdate(operation);
  while (!terminalOperationPhases.has(operation.phase)) {
    await new Promise((resolve) => window.setTimeout(resolve, 750));
    operation = await getOperation(initial.id);
    onUpdate(operation);
  }
  return operation;
}

function publicEnvironmentUrl(publicAddress: string, remotePort: number): string {
  const address = publicAddress.trim().replace(/^\[(.*)\]$/, "$1");
  const host = address.includes(":") ? `[${address}]` : address;
  return `https://${host}:${remotePort}/`;
}

function EnvironmentCredentialsPanel({ environmentId }: { environmentId: string }) {
  const { t } = useTranslation();
  const [showPassword, setShowPassword] = useState(false);
  const [copied, setCopied] = useState<"username" | "password" | null>(null);
  const credentials = useQuery({
    queryKey: ["environment-credentials", environmentId],
    queryFn: () => getEnvironmentCredentials(environmentId),
    staleTime: Infinity,
    gcTime: 0,
  });
  const username = credentials.data?.username ?? `webtop-${environmentId}`;

  const copyCredential = async (kind: "username" | "password", value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(kind);
    } catch {
      setCopied(null);
    }
  };

  return <section className="credential-panel" aria-label={t("environments.credentialsTitle")}>
    <span className="credential-title">{t("environments.credentialsTitle")}</span>
    <div className="credential-row">
      <span>{t("environments.username")}</span>
      <code className="credential-value">{username}</code>
      <div className="credential-actions">
        <button type="button" className="secondary-button small-button" onClick={() => void copyCredential("username", username)}>
          {t(copied === "username" ? "environments.copied" : "environments.copyUsername")}
        </button>
      </div>
    </div>
    <div className="credential-row">
      <span>{t("environments.password")}</span>
      {credentials.data ? <input
        className="credential-value credential-password"
        aria-label={t("environments.password")}
        type={showPassword ? "text" : "password"}
        value={credentials.data.password}
        readOnly
      /> : <span className="credential-loading">
        {t(credentials.isError ? "environments.credentialsUnavailable" : "environments.credentialsLoading")}
      </span>}
      <div className="credential-actions">
        <button type="button" className="secondary-button small-button" disabled={!credentials.data} aria-pressed={showPassword} onClick={() => setShowPassword((visible) => !visible)}>
          {t(showPassword ? "environments.hidePassword" : "environments.showPassword")}
        </button>
        <button type="button" className="secondary-button small-button" disabled={!credentials.data} onClick={() => credentials.data && void copyCredential("password", credentials.data.password)}>
          {t(copied === "password" ? "environments.copied" : "environments.copyPassword")}
        </button>
      </div>
    </div>
    <small>{t("environments.credentialsHint")}</small>
  </section>;
}

export function EnvironmentList({ hostUid, hostGid }: { hostUid: number; hostGid: number }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [creating, setCreating] = useState(false);
  const [pendingCreate, setPendingCreate] = useState<{ spec: EnvironmentSpec; operation: PersistentOperation | null } | null>(null);
  const [deleting, setDeleting] = useState<EnvironmentRecord | null>(null);
  const [showCacheConfirm, setShowCacheConfirm] = useState(false);
  const [copiedEnvironment, setCopiedEnvironment] = useState<string | null>(null);
  const [savingTemplate, setSavingTemplate] = useState<{ environment: EnvironmentRecord; preflight: TemplatePreflight } | null>(null);
  const { activeOperation, beginOperation, finishOperation } = useOperationFeedback();
  const environments = useQuery({ queryKey: ["environments"], queryFn: listEnvironments });
  const images = useQuery({ queryKey: ["official-images"], queryFn: listOfficialImages });
  const templates = useQuery({ queryKey: ["templates"], queryFn: listTemplates });
  const serverSettings = useQuery({ queryKey: ["server-settings"], queryFn: getServerSettings });
  const records = environments.data ?? [];
  const hasPublishedEnvironment = records.some((record) => record.spec.publication.enabled);
  const frpcStatus = useQuery({
    queryKey: ["frpc-status"],
    queryFn: getFrpcStatus,
    enabled: hasPublishedEnvironment,
    refetchInterval: 3000,
  });
  const imagePull = useOfficialImagePull();
  const refresh = () => queryClient.invalidateQueries({ queryKey: ["environments"] });
  const createMutation = useMutation({
    mutationFn: async ({ spec, templateId }: { spec: EnvironmentSpec; templateId: string | null }) => {
      if (!templateId) {
        await createEnvironment(spec);
        return;
      }
      const operation = await createEnvironmentFromTemplate(templateId, spec);
      trackOperation(operation, "restore_template", { displayName: spec.name });
      const completed = await waitForOperation(operation, (current) => {
        setPendingCreate((pending) => pending?.spec.name === spec.name ? { ...pending, operation: current } : pending);
      });
      forgetOperation(operation.id);
      if (completed.phase !== "succeeded") throw completed.error ?? { code: "INTERNAL" };
    },
    onMutate: ({ spec }) => {
      setCreating(false);
      setPendingCreate({ spec, operation: null });
    },
    onSuccess: async () => {
      await refresh();
      setPendingCreate(null);
    },
    onError: () => setPendingCreate(null),
  });
  const actionMutation = useMutation({
    mutationFn: ({ id, action }: { id: string; action: "start" | "stop" | "restart"; operationId: string }) => runEnvironmentAction(id, action),
    onSuccess: refresh,
    onSettled: (_result, _error, variables) => finishOperation(variables.operationId),
  });
  const deleteMutation = useMutation({
    mutationFn: ({ environment, deleteData }: { environment: EnvironmentRecord; deleteData: boolean }) => removeEnvironment(environment.id, environment.name, deleteData),
    onMutate: () => setDeleting(null),
    onSuccess: refresh,
  });
  const publicationMutation = useMutation({
    mutationFn: ({ id, action }: { id: string; action: "publish" | "unpublish"; operationId: string }) =>
      runEnvironmentPublicationAction(id, action),
    onSuccess: refresh,
    onSettled: (_result, _error, variables) => finishOperation(variables.operationId),
  });
  const cachePrune = useMutation({
    mutationFn: ({ operationId: _operationId }: { operationId: string }) => pruneImageCache(),
    onSuccess: async () => {
      setShowCacheConfirm(false);
      await images.refetch();
    },
    onSettled: (_result, _error, variables) => finishOperation(variables.operationId),
  });
  const openDirectory = useMutation({ mutationFn: openEnvironmentDataDirectory });
  const openLocal = useMutation({ mutationFn: openLocalEnvironment });
  const openPublic = useMutation({ mutationFn: openPublicEnvironment });
  const templatePreflight = useMutation({
    mutationFn: (environment: EnvironmentRecord) => getTemplatePreflight(environment.id),
    onSuccess: (preflight, environment) => setSavingTemplate({ environment, preflight }),
  });
  const templateSave = useMutation({
    mutationFn: ({ environment, name, confirmSpace }: { environment: EnvironmentRecord; name: string; confirmSpace: boolean }) =>
      createTemplate({
        environmentId: environment.id,
        name,
        confirmedSensitiveData: true,
        confirmedSpaceWarning: confirmSpace,
      }),
    onSuccess: (operation) => {
      trackOperation(operation, "create_template");
      setSavingTemplate(null);
      void queryClient.invalidateQueries({ queryKey: ["templates"] });
    },
  });
  const publicationAvailable = Boolean(
    serverSettings.data?.frpsHost.trim() && serverSettings.data.publicIp.trim(),
  );

  const copyPublicUrl = async (environmentId: string, url: string) => {
    try {
      await navigator.clipboard.writeText(url);
      setCopiedEnvironment(environmentId);
    } catch {
      setCopiedEnvironment(null);
    }
  };

  return (
    <section aria-labelledby="environment-title">
      <header className="page-header">
        <div><p className="eyebrow">LOCAL DOCKER</p><h2 id="environment-title">{t("environments.title")}</h2><p>{t("environments.count", { count: records.length })}</p></div>
        <button type="button" className={createMutation.isPending ? "primary-button is-working" : "primary-button"} disabled={createMutation.isPending} onClick={() => setCreating(true)}>＋ {t(createMutation.isPending ? "environments.creating" : "common.create")}</button>
      </header>
      <div className="security-banner warning-banner"><span aria-hidden="true">!</span><div><strong>{t("security.internetRiskTitle")}</strong><p>{t("security.internetRiskHint")}</p></div></div>
      {records.length === 0 && !pendingCreate && !environments.isLoading ? <div className="empty-state"><div className="empty-glyph" aria-hidden="true">▦</div><p>{t("environments.empty")}</p></div> : null}
      <div className="environment-grid">
        {pendingCreate ? <article className="environment-card creating-environment-card" aria-busy="true">
          <header><div className="status-dot status-dot-pending" /><div><h3>{pendingCreate.spec.name}</h3><code>{pendingCreate.spec.image}</code></div></header>
          <dl><div><dt>Status</dt><dd>{t("environments.creating")}</dd></div><div><dt>Endpoint</dt><dd>—</dd></div></dl>
          <p className="muted compact">{t("environments.creatingHint")}</p>
          <OperationConsole
            label={t("environments.creationOutput")}
            status={pendingCreate.operation
              ? `${t(`templates.phases.${pendingCreate.operation.phase}`)}${pendingCreate.operation.progressPercent === null ? "" : ` · ${pendingCreate.operation.progressPercent}%`}`
              : t("templates.phases.queued")}
            lines={pendingCreate.operation?.logLines}
            emptyMessage={t("environments.waitingForCreationOutput")}
          />
        </article> : null}
        {records.map((environment) => {
          const pendingAction = actionMutation.isPending && actionMutation.variables?.id === environment.id
            ? actionMutation.variables.action
            : null;
          const pendingPublication = publicationMutation.isPending && publicationMutation.variables?.id === environment.id
            ? publicationMutation.variables.action
            : null;
          const pendingDeletion = deleteMutation.isPending && deleteMutation.variables?.environment.id === environment.id;
          const remotePort = environment.spec.publication.remotePort;
          const publicUrl = environment.desiredRunning
            && environment.spec.publication.enabled
            && remotePort
            && serverSettings.data?.publicIp
            ? publicEnvironmentUrl(serverSettings.data.publicIp, remotePort)
            : null;
          return <article className={pendingDeletion ? "environment-card deleting-environment-card" : "environment-card"} key={environment.id} aria-busy={pendingDeletion} aria-disabled={pendingDeletion} inert={pendingDeletion ? true : undefined}>
            <header><div className={environment.desiredRunning ? "status-dot online" : "status-dot"} /><div><h3>{environment.name}</h3><code>{environment.spec.image}</code></div></header>
            <dl><div><dt>Status</dt><dd>{t(environment.desiredRunning ? "environments.running" : "environments.stopped")}</dd></div><div><dt>Endpoint</dt><dd>{environment.localPort ? t("environments.localAddress", { port: environment.localPort }) : "—"}</dd></div></dl>
            <p className="muted compact">{t("environments.selfSigned")}</p>
            {publicUrl ? <div className="public-endpoint">
              <span>{t("environments.publicAddress")}</span>
              <a href={publicUrl} onClick={(event) => { event.preventDefault(); openPublic.mutate(environment.id); }}>{publicUrl}</a>
              <button type="button" className="secondary-button small-button" onClick={() => void copyPublicUrl(environment.id, publicUrl)}>{t(copiedEnvironment === environment.id ? "environments.copiedPublicAddress" : "environments.copyPublicAddress")}</button>
              <small className={frpcStatus.data?.connected ? "public-ready" : "public-pending"}>{t(frpcStatus.data?.connected ? "environments.publicReady" : "environments.publicNeedsFrpc")}</small>
            </div> : null}
            <EnvironmentCredentialsPanel environmentId={environment.id} />
            <footer>
              {environment.localPort && environment.desiredRunning ? <button type="button" className={openLocal.isPending && openLocal.variables === environment.localPort ? "primary-button small-button is-working" : "primary-button small-button"} disabled={openLocal.isPending} onClick={() => openLocal.mutate(environment.localPort!)}>{t("common.open")}</button> : null}
              <button type="button" className="secondary-button small-button" disabled={openDirectory.isPending} onClick={() => openDirectory.mutate(environment.id)}>{t("environments.openDataDirectory")}</button>
              <button type="button" className={pendingPublication ? "secondary-button small-button is-working" : "secondary-button small-button"} disabled={Boolean(activeOperation) || (!environment.spec.publication.enabled && !publicationAvailable)} onClick={() => {
                const action = environment.spec.publication.enabled ? "unpublish" : "publish";
                if (action === "publish" && !window.confirm(t("environments.publishConfirm"))) return;
                const operationId = beginOperation(action, environment.name);
                if (operationId) publicationMutation.mutate({ id: environment.id, action, operationId });
              }}>{t(pendingPublication ? `environments.${pendingPublication}ing` : environment.spec.publication.enabled ? "environments.unpublish" : "environments.publish")}</button>
              <button type="button" className={pendingAction && pendingAction !== "restart" ? "secondary-button small-button is-working" : "secondary-button small-button"} disabled={Boolean(activeOperation)} onClick={() => {
                const action = environment.desiredRunning ? "stop" : "start";
                const kind: OperationKind = action === "start" ? "environmentStart" : "environmentStop";
                const operationId = beginOperation(kind, environment.name);
                if (operationId) actionMutation.mutate({ id: environment.id, action, operationId });
              }}>{t(pendingAction && pendingAction !== "restart" ? `environments.${pendingAction}ing` : environment.desiredRunning ? "common.stop" : "common.start")}</button>
              <button type="button" className={pendingAction === "restart" ? "secondary-button small-button is-working" : "secondary-button small-button"} disabled={Boolean(activeOperation) || !environment.desiredRunning} onClick={() => {
                const operationId = beginOperation("environmentRestart", environment.name);
                if (operationId) actionMutation.mutate({ id: environment.id, action: "restart", operationId });
              }}>{t(pendingAction === "restart" ? "environments.restarting" : "common.restart")}</button>
              <button type="button" className={templatePreflight.isPending && templatePreflight.variables?.id === environment.id ? "secondary-button small-button is-working" : "secondary-button small-button"} disabled={Boolean(activeOperation) || environment.desiredRunning || templatePreflight.isPending} title={environment.desiredRunning ? t("templates.stopRequired") : undefined} onClick={() => templatePreflight.mutate(environment)}>{t("templates.save")}</button>
              <button type="button" className={pendingDeletion ? "text-danger environment-delete-working" : "text-danger"} disabled={deleteMutation.isPending || Boolean(activeOperation)} onClick={() => setDeleting(environment)}>{t(pendingDeletion ? "environments.deleting" : "common.delete")}</button>
            </footer>
          </article>;
        })}
      </div>
      {createMutation.isError ? <div className="toast-error" role="alert">{t("environments.createFailed")}</div> : null}
      {deleteMutation.isError ? <div className="toast-error" role="alert">{t("environments.deleteFailed")}</div> : null}
      {publicationMutation.isError ? <div className="toast-error" role="alert">{t("environments.publicationFailed")}</div> : null}
      {openDirectory.isError || openLocal.isError || openPublic.isError ? <div className="toast-error" role="alert">{t("environments.openFailed")}</div> : null}
      {templatePreflight.isError || templateSave.isError ? <div className="toast-error" role="alert">{t("templates.saveFailed")}</div> : null}
      <CreateEnvironmentDialog
        open={creating}
        pending={createMutation.isPending}
        onClose={() => setCreating(false)}
        onSubmit={(spec: EnvironmentSpec) => {
          const templateId = templates.data?.find((template) => template.imageReference === spec.image)?.id ?? null;
          createMutation.mutate({ spec, templateId });
        }}
        hostUid={hostUid}
        hostGid={hostGid}
        officialImages={images.data ?? []}
        templates={templates.data ?? []}
        imagesLoading={images.isLoading || templates.isLoading}
        pullingImage={imagePull.isPending ? imagePull.reference : null}
        pullProgress={imagePull.latest}
        pullLogs={imagePull.logs}
        pullCancelling={imagePull.isCancelling}
        pullCancelled={imagePull.outcome === "cancelled"}
        pullFailed={imagePull.isError}
        cachePruning={cachePrune.isPending}
        cachePruneResult={cachePrune.data ?? null}
        cachePruneFailed={cachePrune.isError}
        publicationAvailable={publicationAvailable}
        onPullImage={imagePull.start}
        onCancelPull={() => void imagePull.cancel()}
        onClearCache={() => { cachePrune.reset(); setShowCacheConfirm(true); }}
      />
      <ImageCachePruneDialog open={showCacheConfirm} pending={cachePrune.isPending} onClose={() => setShowCacheConfirm(false)} onConfirm={() => {
        const operationId = beginOperation("cachePrune", t("images.cacheTarget"));
        if (operationId) cachePrune.mutate({ operationId });
      }} />
      <DeleteEnvironmentDialog key={deleting?.id ?? "closed"} environment={deleting} onClose={() => setDeleting(null)} onConfirm={(deleteData) => {
        if (!deleting) return;
        deleteMutation.mutate({ environment: deleting, deleteData });
      }} />
      {savingTemplate ? <SaveTemplateDialog environment={savingTemplate.environment} preflight={savingTemplate.preflight} pending={templateSave.isPending} onClose={() => setSavingTemplate(null)} onConfirm={(name, confirmSpace) => templateSave.mutate({ environment: savingTemplate.environment, name, confirmSpace })} /> : null}
    </section>
  );
}
