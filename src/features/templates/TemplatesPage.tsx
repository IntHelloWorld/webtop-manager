import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  cancelOperation,
  cancelTemplateTransfer,
  checkTemplateSources,
  discardTemplateExportDestination,
  discardTemplateStaging,
  getOperation,
  getTemplateImportPreflight,
  listTemplates,
  removeTemplate,
  saveTemplateExport,
  selectTemplateExport,
  selectTemplateImport,
  stageTemplateImport,
  startTemplateExport,
  startTemplateImport,
} from "../../lib/api";
import { forgetOperation, trackOperation, trackedOperations, type TrackedOperation } from "../../lib/persistentOperations";
import type { PersistentOperation, TemplateImportPreflight, TemplateRecord } from "../../lib/types";
import { OperationConsole } from "../../components/OperationConsole";

const outputClearDelay = 3_000;

interface ActiveTransfer {
  id: string;
  kind: "export" | "import";
  templateId: string | null;
  stopping: boolean;
}

interface ConfirmedImport {
  value: TemplateImportPreflight;
  name: string;
}

function bytes(value: number): string {
  if (value < 1024 ** 2) return `${(value / 1024).toFixed(0)} KiB`;
  if (value < 1024 ** 3) return `${(value / 1024 ** 2).toFixed(1)} MiB`;
  return `${(value / 1024 ** 3).toFixed(2)} GiB`;
}

function shortDigest(value: string | null | undefined): string {
  if (!value) return "—";
  return value.length > 24 ? `${value.slice(0, 18)}…${value.slice(-6)}` : value;
}

function ImportDialog({ value, pending, onClose, onConfirm }: {
  value: TemplateImportPreflight;
  pending: boolean;
  onClose: () => void;
  onConfirm: (name: string) => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(value.nameConflict ? `${value.manifest.name} (imported)` : value.manifest.name);
  const [sensitive, setSensitive] = useState(false);
  const [untrusted, setUntrusted] = useState(false);
  return <div className="dialog-backdrop" role="presentation"><section className="dialog" role="dialog" aria-modal="true" aria-labelledby="import-title">
    <header className="dialog-header"><div><p className="eyebrow">WTMPL v{value.manifest.schemaVersion}</p><h2 id="import-title">{t("templates.importTitle")}</h2></div><button type="button" className="icon-button" aria-label={t("common.close")} disabled={pending} onClick={onClose}>×</button></header>
    <dl className="preflight-grid"><div><dt>{t("templates.platform")}</dt><dd>{value.manifest.platform}</dd></div><div><dt>{t("templates.imagePayload")}</dt><dd>{bytes(value.manifest.imagePayload.sizeBytes)}</dd></div><div><dt>{t("templates.configPayload")}</dt><dd>{bytes(value.manifest.configPayload.sizeBytes)}</dd></div><div><dt>SHA-256</dt><dd title={value.manifest.configPayload.sha256}>{shortDigest(value.manifest.configPayload.sha256)}</dd></div></dl>
    <label className="field"><span>{t("templates.localName")}</span><input value={name} maxLength={64} onChange={(event) => setName(event.target.value)} autoFocus /></label>
    {value.nameConflict ? <div className="warning">{t("templates.nameConflict")}</div> : null}
    <label className="check-row"><input type="checkbox" checked={sensitive} onChange={(event) => setSensitive(event.target.checked)} /><span>{t("templates.importSensitiveConfirm")}<small>{t("templates.unencryptedWarning")}</small></span></label>
    <label className="check-row"><input type="checkbox" checked={untrusted} onChange={(event) => setUntrusted(event.target.checked)} /><span>{t("templates.untrustedConfirm")}<small>{t("templates.untrustedHint")}</small></span></label>
    <footer className="dialog-actions"><button type="button" className="secondary-button" disabled={pending} onClick={onClose}>{t("common.cancel")}</button><button type="button" className={pending ? "primary-button is-working" : "primary-button"} disabled={pending || !name.trim() || !sensitive || !untrusted} onClick={() => onConfirm(name.trim())}>{t(pending ? "templates.importing" : "templates.import")}</button></footer>
  </section></div>;
}

function ImportValidationDialog({ stopping, onStop }: { stopping: boolean; onStop: () => void }) {
  const { t } = useTranslation();
  return <div className="dialog-backdrop" role="presentation"><section className="dialog small" role="dialog" aria-modal="true" aria-labelledby="import-validation-title" aria-busy="true">
    <header className="dialog-header"><div><p className="eyebrow">WTMPL</p><h2 id="import-validation-title">{t("templates.importTitle")}</h2></div><button type="button" className="icon-button" aria-label={t("common.close")} disabled={stopping} onClick={onStop}>×</button></header>
    <div className="validation-pending" role="status"><span className="operation-spinner" aria-hidden="true" /><div><strong>{t("templates.pendingImportName")}</strong><p>{t("templates.validatingHint")}</p></div></div>
    <footer className="dialog-actions"><button type="button" className="danger-outline-button" disabled={stopping} onClick={onStop}>{t(stopping ? "templates.stopping" : "templates.stop")}</button></footer>
  </section></div>;
}

function DeleteTemplateDialog({ template, pending, onClose, onConfirm }: { template: TemplateRecord; pending: boolean; onClose: () => void; onConfirm: () => void }) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  return <div className="dialog-backdrop" role="presentation"><section className="dialog small" role="dialog" aria-modal="true" aria-labelledby="delete-template-title">
    <header className="dialog-header"><div><p className="eyebrow">DESTRUCTIVE</p><h2 id="delete-template-title">{t("templates.deleteTitle")}</h2></div><button type="button" className="icon-button" aria-label={t("common.close")} disabled={pending} onClick={onClose}>×</button></header>
    <p className="dialog-copy">{t("templates.deleteHint")}</p><code className="delete-target">{template.name}</code>
    <label className="field"><span>{t("templates.typeName")}</span><input value={name} onChange={(event) => setName(event.target.value)} autoFocus /></label>
    <footer className="dialog-actions"><button type="button" className="secondary-button" disabled={pending} onClick={onClose}>{t("common.cancel")}</button><button type="button" className="danger-outline-button" disabled={pending || name !== template.name} onClick={onConfirm}>{t("common.delete")}</button></footer>
  </section></div>;
}

export function TemplatesPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const templates = useQuery({ queryKey: ["templates"], queryFn: listTemplates, refetchInterval: 10_000 });
  const [tracked, setTracked] = useState<TrackedOperation[]>(() => trackedOperations());
  const [deleting, setDeleting] = useState<TemplateRecord | null>(null);
  const [importValue, setImportValue] = useState<TemplateImportPreflight | null>(null);
  const [confirmedImport, setConfirmedImport] = useState<ConfirmedImport | null>(null);
  const [pendingExport, setPendingExport] = useState<{ stagingFileId: string; suggestedName: string; templateId: string } | null>(null);
  const [activeTransfer, setActiveTransfer] = useState<ActiveTransfer | null>(null);
  const [error, setError] = useState<string | null>(null);
  const handled = useRef(new Set<string>());
  const clearTimers = useRef(new Map<string, number>());
  useEffect(() => () => {
    for (const timer of clearTimers.current.values()) window.clearTimeout(timer);
  }, []);

  useEffect(() => {
    const refresh = () => setTracked(trackedOperations());
    window.addEventListener("webtop-operations-changed", refresh);
    return () => window.removeEventListener("webtop-operations-changed", refresh);
  }, []);

  const scheduleOperationClear = useCallback((operationId: string) => {
    const previous = clearTimers.current.get(operationId);
    if (previous) window.clearTimeout(previous);
    const timer = window.setTimeout(() => {
      forgetOperation(operationId);
      handled.current.delete(operationId);
      clearTimers.current.delete(operationId);
      setTracked(trackedOperations());
    }, outputClearDelay);
    clearTimers.current.set(operationId, timer);
  }, []);
  const clearOperation = useCallback((operationId: string) => {
    forgetOperation(operationId);
    handled.current.delete(operationId);
    setTracked(trackedOperations());
  }, []);

  const operationQueries = useQueries({
    queries: tracked.map((item) => ({
      queryKey: ["operation", item.id],
      queryFn: () => getOperation(item.id),
      refetchInterval: (query: { state: { data?: PersistentOperation } }) => {
        const phase = query.state.data?.phase;
        return phase && ["succeeded", "failed", "cancelled", "retryable"].includes(phase) ? false : 1000;
      },
    })),
  });
  const activeOperations = operationQueries.map((query) => query.data).filter((value): value is PersistentOperation => Boolean(value));

  useEffect(() => {
    for (const operation of activeOperations) {
      if (!["succeeded", "failed", "cancelled", "retryable"].includes(operation.phase) || handled.current.has(operation.id)) continue;
      handled.current.add(operation.id);
      const trackedItem = tracked.find((item) => item.id === operation.id);
      void (async () => {
        if (operation.phase === "succeeded" && trackedItem?.kind === "export_template") {
          const stagingFileId = operation.result?.stagingFileId;
          const suggestedName = operation.result?.suggestedName;
          if (typeof stagingFileId === "string" && typeof suggestedName === "string") {
            if (!trackedItem.exportDestinationId) {
              setPendingExport({ stagingFileId, suggestedName, templateId: operation.resourceId ?? "" });
              setError(t("templates.exportPending"));
            } else {
              const transferId = crypto.randomUUID();
              setActiveTransfer({ id: transferId, kind: "export", templateId: operation.resourceId, stopping: false });
              try {
                const saved = await saveTemplateExport(
                  stagingFileId,
                  trackedItem.exportDestinationId,
                  transferId,
                  () => undefined,
                );
                if (!saved) setError(t("templates.operationFailed"));
              } catch {
                await discardTemplateExportDestination(trackedItem.exportDestinationId).catch(() => undefined);
                setPendingExport({ stagingFileId, suggestedName, templateId: operation.resourceId ?? "" });
                setError(t("templates.exportPending"));
              } finally {
                setActiveTransfer((current) => current?.id === transferId ? null : current);
              }
            }
          } else if (trackedItem.exportDestinationId) {
            await discardTemplateExportDestination(trackedItem.exportDestinationId).catch(() => undefined);
          }
        }
        if (operation.phase !== "succeeded" && trackedItem?.exportDestinationId) {
          await discardTemplateExportDestination(trackedItem.exportDestinationId).catch(() => undefined);
        }
        if (operation.phase === "failed") {
          setError(t(`errors.${operation.error?.code ?? "INTERNAL"}`, { defaultValue: t("templates.operationFailed") }));
        }
        await queryClient.invalidateQueries({ queryKey: ["templates"] });
        await queryClient.invalidateQueries({ queryKey: ["environments"] });
        if (trackedItem?.kind === "export_template") {
          scheduleOperationClear(operation.id);
        } else if (trackedItem?.kind === "import_template") {
          if (operation.phase === "succeeded") clearOperation(operation.id);
          else scheduleOperationClear(operation.id);
        } else {
          scheduleOperationClear(operation.id);
        }
      })();
    }
  }, [activeOperations, clearOperation, queryClient, scheduleOperationClear, t, tracked]);

  const begin = (operation: PersistentOperation, kind?: string, details: Pick<TrackedOperation, "exportDestinationId" | "displayName"> = {}) => {
    trackOperation(operation, kind, details);
    setTracked(trackedOperations());
  };
  const sourceCheck = useMutation({ mutationFn: () => checkTemplateSources(), onSuccess: (operation) => begin(operation, "source_check") });
  const exportMutation = useMutation({
    mutationFn: async (template: TemplateRecord) => {
      const destinationId = await selectTemplateExport(`${template.name}.wtmpl`);
      if (!destinationId) return null;
      try {
        const operation = await startTemplateExport(template.id);
        return { destinationId, operation };
      } catch (error) {
        await discardTemplateExportDestination(destinationId).catch(() => undefined);
        throw error;
      }
    },
    onSuccess: (value) => {
      if (value) begin(value.operation, "export_template", { exportDestinationId: value.destinationId });
    },
    onError: () => {
      setError(t("templates.operationFailed"));
    },
  });
  const selectImport = useMutation({
    mutationFn: async () => {
      const sourceId = await selectTemplateImport();
      if (!sourceId) return null;
      const transferId = crypto.randomUUID();
      setActiveTransfer({ id: transferId, kind: "import", templateId: null, stopping: false });
      try {
        const staging = await stageTemplateImport(sourceId, transferId, () => undefined);
        if (!staging) return null;
        return await getTemplateImportPreflight(staging);
      } finally {
        setActiveTransfer((current) => current?.id === transferId ? null : current);
      }
    },
    onSuccess: (value) => { if (value) setImportValue(value); },
    onError: () => {
      setError(t("templates.invalidPackage"));
    },
  });
  const importMutation = useMutation({
    mutationFn: ({ value, name }: { value: TemplateImportPreflight; name: string }) => startTemplateImport({ stagingFileId: value.stagingFileId, name, confirmedSensitiveData: true, confirmedUntrustedImage: true }),
    onMutate: (value) => {
      setConfirmedImport(value);
      setImportValue(null);
    },
    onSuccess: (operation, value) => {
      queryClient.setQueryData(["operation", operation.id], operation);
      begin(operation, "import_template", { displayName: value.name });
      setConfirmedImport(null);
    },
    onError: (_error, value) => {
      setConfirmedImport(null);
      setImportValue(value.value);
      setError(t("templates.operationFailed"));
    },
  });
  const retryExport = useMutation({
    mutationFn: async (value: NonNullable<typeof pendingExport>) => {
      const destinationId = await selectTemplateExport(value.suggestedName);
      if (!destinationId) return false;
      const transferId = crypto.randomUUID();
      setActiveTransfer({ id: transferId, kind: "export", templateId: value.templateId, stopping: false });
      try {
        return await saveTemplateExport(value.stagingFileId, destinationId, transferId, () => undefined);
      } catch (error) {
        await discardTemplateExportDestination(destinationId).catch(() => undefined);
        throw error;
      } finally {
        setActiveTransfer((current) => current?.id === transferId ? null : current);
      }
    },
    onSuccess: (saved) => {
      if (!saved) return;
      setPendingExport(null);
    },
    onError: () => setError(t("templates.exportPending")),
  });
  const stopOperation = useMutation({
    mutationFn: (operation: PersistentOperation) => cancelOperation(operation.id),
    onSuccess: (operation) => queryClient.setQueryData(["operation", operation.id], operation),
    onError: () => setError(t("templates.stopFailed")),
  });
  const stopTransfer = useMutation({
    mutationFn: async (transfer: ActiveTransfer) => {
      setActiveTransfer((current) => current?.id === transfer.id ? { ...current, stopping: true } : current);
      await cancelTemplateTransfer(transfer.id);
    },
    onError: (_error, transfer) => {
      setActiveTransfer((current) => current?.id === transfer.id ? { ...current, stopping: false } : current);
      setError(t("templates.stopFailed"));
    },
  });
  const deleteMutation = useMutation({
    mutationFn: (template: TemplateRecord) => removeTemplate(template.id, template.name),
    onSuccess: (operation) => { begin(operation, "delete_template"); setDeleting(null); },
    onError: (value: { code?: string }) => setError(t(value?.code === "TEMPLATE_DEPENDENCY" ? "templates.deleteDependency" : "templates.operationFailed")),
  });
  const records = templates.data ?? [];
  const importOperations = activeOperations.filter((operation) => operation.kind === "import_template");
  const importOperation = importOperations.at(-1) ?? null;
  const importTracked = importOperation ? tracked.find((item) => item.id === importOperation.id) : null;
  const importTransfer = activeTransfer?.kind === "import" ? activeTransfer : null;
  const showImportCard = Boolean(confirmedImport || importOperation);
  const importCardName = importTracked?.displayName ?? confirmedImport?.name ?? t("templates.pendingImportName");
  const importCardPreflight = confirmedImport?.value ?? null;
  const importStopping = importOperation?.phase === "rolling_back";
  const importRunning = Boolean(confirmedImport || (importOperation && !["succeeded", "failed", "cancelled", "retryable"].includes(importOperation.phase)));
  const busy = Boolean(activeTransfer) || importMutation.isPending || activeOperations.some((operation) => !["succeeded", "failed", "cancelled", "retryable"].includes(operation.phase));

  return <section aria-labelledby="templates-title">
    <header className="page-header"><div><p className="eyebrow">PORTABLE / OFFLINE</p><h2 id="templates-title">{t("templates.title")}</h2><p>{t("templates.count", { count: records.length })}</p></div><div className="page-actions"><button type="button" className="secondary-button" disabled={busy || sourceCheck.isPending} onClick={() => sourceCheck.mutate()}>{t("templates.checkUpdates")}</button><button type="button" className="primary-button" disabled={busy || selectImport.isPending} onClick={() => selectImport.mutate()}>＋ {t(selectImport.isPending ? "templates.importing" : "templates.import")}</button></div></header>
    <div className="security-banner warning-banner"><span aria-hidden="true">!</span><div><strong>{t("templates.securityTitle")}</strong><p>{t("templates.securityHint")}</p></div></div>
    {pendingExport ? <div className="security-banner"><span aria-hidden="true">↓</span><div><strong>{t("templates.exportPendingTitle")}</strong><p>{t("templates.exportPending")}</p><div className="service-actions"><button type="button" className={retryExport.isPending ? "primary-button small-button is-working" : "primary-button small-button"} disabled={retryExport.isPending} onClick={() => retryExport.mutate(pendingExport)}>{t("templates.saveExport")}</button><button type="button" className="secondary-button small-button" disabled={retryExport.isPending} onClick={() => { void discardTemplateStaging(pendingExport.stagingFileId).then(() => setPendingExport(null)); }}>{t("templates.discardExport")}</button></div></div></div> : null}
    {!records.length && !templates.isLoading && !showImportCard ? <div className="empty-state"><div className="empty-glyph" aria-hidden="true">◇</div><p>{t("templates.empty")}</p></div> : null}
    <div className="template-grid">
      {showImportCard ? <article className="template-card importing-template-card" aria-busy={importRunning}>
        <header><div><p className="eyebrow">IMPORTING WTMPL</p><h3>{importCardName}</h3><code>{t("templates.pendingImportHint")}</code></div><span className="status-badge updated">{t(importStopping ? "templates.stopping" : "templates.importing")}</span></header>
        {importCardPreflight ? <dl><div><dt>{t("templates.platform")}</dt><dd>{importCardPreflight.manifest.platform}</dd></div><div><dt>{t("templates.imagePayload")}</dt><dd>{bytes(importCardPreflight.manifest.imagePayload.sizeBytes)}</dd></div><div><dt>{t("templates.configPayload")}</dt><dd>{bytes(importCardPreflight.manifest.configPayload.sizeBytes)}</dd></div><div><dt>SHA-256</dt><dd>{shortDigest(importCardPreflight.manifest.configPayload.sha256)}</dd></div></dl> : null}
        <OperationConsole
          label={t("templates.operationOutput")}
          status={importOperation ? `${t(`templates.phases.${importOperation.phase}`)}${importOperation.progressPercent === null ? "" : ` · ${importOperation.progressPercent}%`}` : t("templates.phases.queued")}
          lines={importOperation?.logLines}
          emptyMessage={t("templates.waitingForOutput")}
        />
        {importOperation && importOperation.cancellable && !["succeeded", "failed", "cancelled", "retryable"].includes(importOperation.phase) ? <footer><button type="button" className="danger-outline-button small-button" disabled={stopOperation.isPending && stopOperation.variables?.id === importOperation.id} onClick={() => stopOperation.mutate(importOperation)}>{t(stopOperation.isPending && stopOperation.variables?.id === importOperation.id ? "templates.stopping" : "templates.stop")}</button></footer> : null}
      </article> : null}
      {records.map((template) => {
      const exportOperations = activeOperations.filter((operation) => operation.kind === "export_template" && operation.resourceId === template.id);
      const exportOperation = exportOperations.at(-1) ?? null;
      const exporting = exportOperations.some((operation) => !["succeeded", "failed", "cancelled", "retryable"].includes(operation.phase));
      const exportTransfer = activeTransfer?.kind === "export" && activeTransfer.templateId === template.id ? activeTransfer : null;
      return <article className="template-card" key={template.id}>
      <header><div><h3>{template.name}</h3><code>{template.imageReference}</code></div><div className="badge-stack"><span className={`status-badge ${template.integrity === "complete" ? "installed" : "broken"}`}>{t(`templates.integrity.${template.integrity}`)}</span>{template.sourceCheck.status === "updated" ? <span className="status-badge updated">{t("templates.sourceUpdated")}</span> : null}{template.trust === "imported_untrusted" ? <span className="status-badge warning-badge">{t("templates.imported")}</span> : null}</div></header>
      <dl><div><dt>{t("templates.source")}</dt><dd title={template.officialSource?.reference ?? undefined}>{template.officialSource?.reference ?? t("templates.derivedSource")}</dd></div><div><dt>{t("templates.version")}</dt><dd title={template.officialSource?.digest ?? template.imageId}>{template.officialSource?.buildVersion ?? shortDigest(template.officialSource?.digest ?? template.imageId)}</dd></div><div><dt>{t("templates.parent")}</dt><dd>{template.parentTemplateId ? shortDigest(template.parentTemplateId) : "—"}</dd></div><div><dt>{t("templates.imageDelta")}</dt><dd>{bytes(template.systemDeltaBytes)}</dd></div><div><dt>{t("templates.snapshotSize")}</dt><dd>{bytes(template.snapshotSizeBytes)}</dd></div><div><dt>{t("templates.createdAt")}</dt><dd>{new Date(template.createdAt).toLocaleString()}</dd></div></dl>
      {template.sourceCheck.status === "unavailable" ? <p className="muted compact">{t("templates.sourceUnavailable")}</p> : null}
      <footer><div className="template-transfer-action export-action"><button type="button" className={exporting || exportTransfer ? "secondary-button small-button is-working" : "secondary-button small-button"} disabled={busy || exportMutation.isPending} onClick={() => exportMutation.mutate(template)}>{t(exporting || exportTransfer ? "templates.exporting" : "templates.export")}</button>{exportTransfer ? <button type="button" className="danger-outline-button small-button template-stop-button" disabled={exportTransfer.stopping || stopTransfer.isPending} onClick={() => stopTransfer.mutate(exportTransfer)}>{t(exportTransfer.stopping ? "templates.stopping" : "templates.stop")}</button> : exportOperation && exportOperation.cancellable && !["succeeded", "failed", "cancelled", "retryable"].includes(exportOperation.phase) ? <button type="button" className="danger-outline-button small-button template-stop-button" disabled={stopOperation.isPending && stopOperation.variables?.id === exportOperation.id} onClick={() => stopOperation.mutate(exportOperation)}>{t(stopOperation.isPending && stopOperation.variables?.id === exportOperation.id ? "templates.stopping" : "templates.stop")}</button> : null}</div><button type="button" className="text-danger" disabled={busy} onClick={() => setDeleting(template)}>{t("common.delete")}</button></footer>
    </article>;
    })}</div>
    {error ? <div className="toast-error" role="alert"><span>{error}</span><button type="button" className="icon-button" aria-label={t("common.close")} onClick={() => setError(null)}>×</button></div> : null}
    {importTransfer ? <ImportValidationDialog stopping={importTransfer.stopping} onStop={() => stopTransfer.mutate(importTransfer)} /> : null}
    {importValue ? <ImportDialog value={importValue} pending={importMutation.isPending} onClose={() => { void discardTemplateStaging(importValue.stagingFileId); setImportValue(null); }} onConfirm={(name) => importMutation.mutate({ value: importValue, name })} /> : null}
    {deleting ? <DeleteTemplateDialog template={deleting} pending={deleteMutation.isPending} onClose={() => setDeleting(null)} onConfirm={() => deleteMutation.mutate(deleting)} /> : null}
  </section>;
}
