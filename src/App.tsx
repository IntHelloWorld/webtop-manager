import { lazy, Suspense, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { AppShell, type Section } from "./components/AppShell";
import { Diagnostics } from "./features/diagnostics/Diagnostics";
import { EnvironmentList } from "./features/environments/EnvironmentList";
import { ImageCatalog } from "./features/images/ImageCatalog";
import { ServerSettingsPage } from "./features/settings/ServerSettingsPage";
import { GuidePage } from "./features/guide/GuidePage";
import { OperationFeedbackProvider, useOperationFeedback } from "./components/OperationFeedbackContext";
import { initializeBackend } from "./lib/api";
import type { ApiError } from "./lib/types";

const TemplatesPage = lazy(() => import("./features/templates/TemplatesPage").then((module) => ({ default: module.TemplatesPage })));

function AppContent() {
  const { t } = useTranslation();
  const [section, setSection] = useState<Section>("environments");
  const queryClient = useQueryClient();
  const { activeOperation } = useOperationFeedback();
  const backend = useQuery({ queryKey: ["backend"], queryFn: initializeBackend, retry: false });

  useEffect(() => {
    const unlisten = listen("resource-changed", () => {
      void queryClient.invalidateQueries({ queryKey: ["environments"] });
      void queryClient.invalidateQueries({ queryKey: ["official-images"] });
      void queryClient.invalidateQueries({ queryKey: ["server-settings"] });
      void queryClient.invalidateQueries({ queryKey: ["frpc-status"] });
      void queryClient.invalidateQueries({ queryKey: ["templates"] });
    });
    return () => { void unlisten.then((dispose) => dispose()); };
  }, [queryClient]);

  if (backend.data?.state !== "ready") {
    return <Diagnostics status={backend.data} error={(backend.error as ApiError | null) ?? null} loading={backend.isLoading || backend.isFetching} onRetry={() => void backend.refetch()} />;
  }

  return (
    <AppShell section={section} onSectionChange={setSection} activeOperation={activeOperation}>
      {section === "environments" ? <EnvironmentList hostUid={backend.data.hostUid} hostGid={backend.data.hostGid} /> : null}
      {section === "guide" ? <GuidePage onNavigate={setSection} /> : null}
      {section === "images" ? <ImageCatalog /> : null}
      {section === "settings" ? <ServerSettingsPage /> : null}
      {section === "templates" ? <Suspense fallback={<p className="muted">{t("common.loading")}</p>}><TemplatesPage /></Suspense> : null}
    </AppShell>
  );
}

export default function App() {
  return <OperationFeedbackProvider><AppContent /></OperationFeedbackProvider>;
}
