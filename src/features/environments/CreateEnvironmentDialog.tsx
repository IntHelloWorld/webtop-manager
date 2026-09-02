import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect } from "react";
import { Controller, useFieldArray, useForm, type Control, type FieldPath } from "react-hook-form";
import { useTranslation } from "react-i18next";
import { z } from "zod";
import { HelpTip } from "../../components/HelpTip";
import { AppSelect, type AppSelectOption } from "../../components/AppSelect";
import type { EnvironmentSpec, ImageCachePruneResult, ImagePullProgress, OfficialImage, TemplateRecord } from "../../lib/types";
import { ImagePullStatus } from "../images/ImagePullStatus";
import { getOfficialOption, officialOptionGroups, officialWebtopOptions, valuesForOption } from "./officialWebtopOptions";

const officialOptionKeys = new Set(officialWebtopOptions.map((option) => option.key));

const schema = z.object({
  name: z.string().trim().min(1).max(64),
  image: z.string().trim().min(1).refine((value) => !/\s/.test(value)),
  cpuLimit: z.enum(["unlimited", "1", "2", "4", "8", "16", "32", "64"]),
  memoryGiB: z.enum(["unlimited", "1", "2", "4", "8", "16", "32", "64", "128", "256"]),
  shmGiB: z.enum(["0.5", "1", "2", "4", "8"]),
  timezone: z.string().trim().min(1).max(64),
  locale: z.string().trim().min(1).max(64),
  resolution: z.string().regex(/^(auto|\d{3,5}x\d{3,5})$/),
  wayland: z.enum(["auto", "true", "false"]),
  gpu: z.enum(["disabled", "dri", "nvidia"]),
  audio: z.enum(["true", "false"]),
  clipboard: z.enum(["true", "false"]),
  fileTransfer: z.enum(["upload,download", "upload", "download", "none"]),
  publication: z.enum(["false", "true"]),
  dockerSocket: z.enum(["false", "true"]),
  seccomp: z.enum(["default", "unconfined"]),
  mounts: z.array(z.object({
    hostPath: z.string().trim().startsWith("/").refine((value) => value !== "/var/run/docker.sock"),
    containerPath: z.string().trim().startsWith("/").refine((value) => value !== "/config" && !value.startsWith("/run/webtop-manager")),
    readOnly: z.enum(["true", "false"]),
  })).max(16),
  officialOptions: z.array(z.object({
    key: z.string().refine((value) => officialOptionKeys.has(value)),
    value: z.string().min(1).max(4096).refine((value) => !value.includes("\0")),
  })).max(officialWebtopOptions.length),
}).superRefine((values, context) => {
  const keys = new Set<string>();
  values.officialOptions.forEach((option, index) => {
    if (keys.has(option.key)) context.addIssue({ code: "custom", path: ["officialOptions", index, "key"], message: "duplicate option" });
    keys.add(option.key);
  });
});

type FormValues = z.infer<typeof schema>;

interface FormSelectProps {
  control: Control<FormValues>;
  name: FieldPath<FormValues>;
  options: AppSelectOption[];
  disabled?: boolean;
  ariaLabel?: string;
  onValueChange?: (value: string) => void;
}

function FormSelect({ control, name, options, disabled, ariaLabel, onValueChange }: FormSelectProps) {
  return (
    <Controller
      control={control}
      name={name}
      render={({ field, fieldState }) => (
        <AppSelect
          ref={field.ref}
          name={field.name}
          value={String(field.value)}
          options={options}
          disabled={disabled}
          ariaLabel={ariaLabel}
          ariaInvalid={fieldState.invalid}
          onBlur={field.onBlur}
          onChange={(value) => {
            field.onChange(value);
            onValueChange?.(value);
          }}
        />
      )}
    />
  );
}

interface CreateEnvironmentDialogProps {
  open: boolean;
  pending: boolean;
  onClose: () => void;
  onSubmit: (spec: EnvironmentSpec) => void;
  hostUid: number;
  hostGid: number;
  officialImages: OfficialImage[];
  templates: TemplateRecord[];
  imagesLoading: boolean;
  pullingImage: string | null;
  pullProgress: ImagePullProgress | null;
  pullLogs: ImagePullProgress[];
  pullCancelling: boolean;
  pullCancelled: boolean;
  pullFailed: boolean;
  cachePruning: boolean;
  cachePruneResult: ImageCachePruneResult | null;
  cachePruneFailed: boolean;
  publicationAvailable: boolean;
  onPullImage: (reference: string) => void;
  onCancelPull: () => void;
  onClearCache: () => void;
}

const defaultValues: FormValues = {
  name: "",
  image: "lscr.io/linuxserver/webtop:ubuntu-mate",
  cpuLimit: "unlimited",
  memoryGiB: "unlimited",
  shmGiB: "1",
  timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || "Etc/UTC",
  locale: "zh_CN.UTF-8",
  resolution: "auto",
  wayland: "auto",
  gpu: "disabled",
  audio: "true",
  clipboard: "true",
  fileTransfer: "upload,download",
  publication: "false",
  dockerSocket: "false",
  seccomp: "default",
  mounts: [],
  officialOptions: [],
};

const timezoneOptions = ["Etc/UTC", "Asia/Shanghai", "Asia/Hong_Kong", "Asia/Taipei", "Asia/Tokyo", "Asia/Seoul", "Asia/Singapore", "Europe/London", "Europe/Berlin", "America/New_York", "America/Chicago", "America/Denver", "America/Los_Angeles"] as const;
const localeOptions = ["en_US.UTF-8", "zh_CN.UTF-8", "ja_JP.UTF-8", "ko_KR.UTF-8", "ar_AE.UTF-8", "ru_RU.UTF-8", "es_MX.UTF-8", "de_DE.UTF-8", "fr_FR.UTF-8", "nl_NL.UTF-8", "it_IT.UTF-8"] as const;

function valuesFromSpec(spec: EnvironmentSpec): FormValues {
  const transfer = spec.display.fileTransferMode === "upload_download"
    ? "upload,download"
    : spec.display.fileTransferMode ?? (spec.display.fileTransfer ? "upload,download" : "none");
  return {
    name: "",
    image: spec.image,
    cpuLimit: (spec.resources.cpuLimit === null ? "unlimited" : String(spec.resources.cpuLimit)) as FormValues["cpuLimit"],
    memoryGiB: (spec.resources.memoryBytes === null ? "unlimited" : String(spec.resources.memoryBytes / 1024 ** 3)) as FormValues["memoryGiB"],
    shmGiB: String(spec.resources.shmBytes / 1024 ** 3) as FormValues["shmGiB"],
    timezone: spec.identity.timezone,
    locale: spec.identity.locale,
    resolution: spec.display.width && spec.display.height ? `${spec.display.width}x${spec.display.height}` : "auto",
    wayland: spec.display.wayland === null ? "auto" : String(spec.display.wayland) as "true" | "false",
    gpu: spec.display.gpu,
    audio: String(spec.display.audio) as "true" | "false",
    clipboard: String(spec.display.clipboard) as "true" | "false",
    fileTransfer: transfer as FormValues["fileTransfer"],
    publication: String(spec.publication.enabled) as "true" | "false",
    dockerSocket: String(spec.security.dockerSocket) as "true" | "false",
    seccomp: spec.security.seccomp,
    mounts: spec.mounts.map((mount) => ({ ...mount, readOnly: String(mount.readOnly) as "true" | "false" })),
    officialOptions: Object.entries(spec.extraEnvironment)
      .filter(([key]) => officialOptionKeys.has(key))
      .map(([key, value]) => ({ key, value })),
  };
}

function parseResolution(resolution: string): { width: number | null; height: number | null } {
  if (resolution === "auto") return { width: null, height: null };
  const [width, height] = resolution.split("x").map(Number);
  return { width, height };
}

function formatCacheBytes(bytes: number): string {
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(0)} KiB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MiB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GiB`;
}

export function CreateEnvironmentDialog({ open, pending, onClose, onSubmit, hostUid, hostGid, officialImages, templates, imagesLoading, pullingImage, pullProgress, pullLogs, pullCancelling, pullCancelled, pullFailed, cachePruning, cachePruneResult, cachePruneFailed, publicationAvailable, onPullImage, onCancelPull, onClearCache }: CreateEnvironmentDialogProps) {
  const { t, i18n } = useTranslation();
  const { register, handleSubmit, watch, reset, setValue, control, formState: { errors } } = useForm<FormValues>({ resolver: zodResolver(schema), defaultValues });
  const mounts = useFieldArray({ control, name: "mounts" });
  const options = useFieldArray({ control, name: "officialOptions" });
  const dockerSocket = watch("dockerSocket");
  const publication = watch("publication");
  const seccomp = watch("seccomp");
  const imageReference = watch("image");
  const selectedOptions = watch("officialOptions");
  const selectedImage = officialImages.find((image) => image.reference === imageReference);
  const selectedTemplate = templates.find((template) => template.imageReference === imageReference);
  const selectionReady = selectedTemplate
    ? selectedTemplate.integrity === "complete"
    : Boolean(selectedImage?.installed);

  useEffect(() => {
    if (!open) return;
    reset({ ...defaultValues, locale: i18n.resolvedLanguage?.startsWith("zh") ? "zh_CN.UTF-8" : "en_US.UTF-8" });
  }, [i18n.resolvedLanguage, open, reset]);

  useEffect(() => {
    if (selectedImage?.waylandOnly) setValue("wayland", "true");
  }, [selectedImage?.waylandOnly, setValue]);

  useEffect(() => {
    if (!open) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !pending) onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose, open, pending]);

  if (!open) return null;

  const appendOfficialOption = () => {
    const selectedKeys = new Set(selectedOptions.map((option) => option.key));
    const next = officialWebtopOptions.find((option) => !selectedKeys.has(option.key));
    if (next) options.append({ key: next.key, value: next.defaultValue });
  };

  const selectOfficialImage = (reference: string) => {
    if (selectedTemplate) {
      const currentName = watch("name");
      reset({
        ...defaultValues,
        name: currentName,
        image: reference,
        locale: i18n.resolvedLanguage?.startsWith("zh") ? "zh_CN.UTF-8" : "en_US.UTF-8",
      });
      return;
    }
    setValue("image", reference, { shouldValidate: true });
  };

  const selectTemplateImage = (template: TemplateRecord) => {
    const currentName = watch("name");
    const templateValues = valuesFromSpec(template.sourceSpec);
    reset({
      ...templateValues,
      name: currentName,
      image: template.imageReference,
      publication: publicationAvailable ? templateValues.publication : "false",
    });
  };

  const submit = (values: FormValues) => {
    const resolution = parseResolution(values.resolution);
    const extraEnvironment = Object.fromEntries(values.officialOptions.map((option) => [option.key, option.value]));
    onSubmit({
      name: values.name,
      image: values.image,
      identity: { uid: hostUid, gid: hostGid, timezone: values.timezone, locale: values.locale },
      resources: {
        cpuLimit: values.cpuLimit === "unlimited" ? null : Number(values.cpuLimit),
        memoryBytes: values.memoryGiB === "unlimited" ? null : Number(values.memoryGiB) * 1024 ** 3,
        shmBytes: Number(values.shmGiB) * 1024 ** 3,
      },
      display: {
        ...resolution,
        wayland: values.wayland === "auto" ? null : values.wayland === "true",
        gpu: values.gpu,
        audio: values.audio === "true",
        clipboard: values.clipboard === "true",
        fileTransfer: values.fileTransfer !== "none",
        fileTransferMode: values.fileTransfer === "upload,download" ? "upload_download" : values.fileTransfer,
      },
      mounts: values.mounts.map((mount) => ({ hostPath: mount.hostPath, containerPath: mount.containerPath, readOnly: mount.readOnly === "true" })),
      security: { dockerSocket: values.dockerSocket === "true", dockerSocketGid: null, privileged: false, seccomp: values.seccomp, devices: [] },
      extraEnvironment,
      publication: {
        enabled: values.publication === "true",
        remotePort: null,
        automaticPort: true,
      },
    });
  };

  return (
    <div className="dialog-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget && !pending) onClose(); }}>
      <section className="dialog dialog-wide" role="dialog" aria-modal="true" aria-labelledby="create-title">
        <header className="dialog-header">
          <div><p className="eyebrow">WEBTOP</p><h2 id="create-title">{t("environments.createTitle")}</h2></div>
          <button type="button" className="icon-button" disabled={pending} onClick={onClose} aria-label={t("common.close")}>×</button>
        </header>
        <form onSubmit={handleSubmit(submit)}>
          <div className="form-grid">
            <label className="field full"><span>{t("environments.name")}</span><input {...register("name")} placeholder={t("environments.namePlaceholder")} aria-invalid={!!errors.name} autoFocus /></label>
            <fieldset className="image-picker full" aria-invalid={!!errors.image} disabled={imagesLoading}>
              <legend><span className="field-label">{t("environments.image")}<HelpTip label={t("environments.image")} text={t("environments.help.image")} /></span></legend>
              <input type="hidden" {...register("image")} />
              <div className="image-source-groups">
                <section className="image-source-group" aria-labelledby="official-image-heading">
                  <header><strong id="official-image-heading">{t("environments.officialImages")}</strong><small>{t("environments.officialImagesHint")}</small></header>
                  {officialImages.length === 0 ? <p className="muted">{t("environments.checkingImages")}</p> : <div className="image-option-list">
                    {officialImages.map((image) => (
                      <div className={image.reference === imageReference ? "image-option selected" : "image-option"} key={image.reference}>
                        <label><input type="radio" name="environment-image-source" checked={image.reference === imageReference} onChange={() => selectOfficialImage(image.reference)} /><span className="image-option-main"><strong>{image.distribution} · {image.desktop}</strong><code>{image.tag}</code></span><span className={image.installed ? "status-badge installed" : "status-badge"}>{t(image.installed ? "images.installed" : "images.notInstalled")}</span></label>
                        {!image.installed ? <button type="button" className="secondary-button small-button" disabled={pullingImage !== null} onClick={() => onPullImage(image.reference)}>{t(pullingImage === image.reference ? "images.pulling" : "images.pull")}</button> : null}
                      </div>
                    ))}
                  </div>}
                </section>
                <section className="image-source-group" aria-labelledby="template-image-heading">
                  <header><strong id="template-image-heading">{t("environments.templateImages")}</strong><small>{t("environments.templateImagesHint")}</small></header>
                  {templates.length === 0 ? <p className="muted">{t("environments.noTemplateImages")}</p> : <div className="image-option-list template-image-options">
                    {templates.map((template) => (
                      <div className={template.imageReference === imageReference ? "image-option selected" : "image-option"} key={template.id}>
                        <label><input type="radio" name="environment-image-source" checked={template.imageReference === imageReference} onChange={() => selectTemplateImage(template)} /><span className="image-option-main"><strong>{template.name}</strong><code>{template.imageReference}</code></span><span className={`status-badge ${template.integrity === "complete" ? "installed" : "broken"}`}>{t(`templates.integrity.${template.integrity}`)}</span></label>
                      </div>
                    ))}
                  </div>}
                </section>
              </div>
              {selectedTemplate ? <small className="image-picker-hint">{t("environments.templateImageReady", { name: selectedTemplate.name })}</small> : selectedImage ? <small className="image-picker-hint">{t(selectedImage.installed ? "environments.imageReady" : "environments.imageNeedsPull")}</small> : null}
              {pullingImage ? <ImagePullStatus latest={pullProgress} logs={pullLogs} isCancelling={pullCancelling} onCancel={onCancelPull} /> : null}
              {!pullingImage && pullCancelled ? <div className="pull-cancelled" role="status"><span>{t("images.pullCancelled")}</span><button type="button" className="secondary-button small-button" disabled={cachePruning} onClick={onClearCache}>{t("images.clearCache")}</button></div> : null}
              {cachePruneResult ? <p className="cache-prune-success" role="status">{t("images.cacheCleared", { count: cachePruneResult.deletedItems, size: formatCacheBytes(cachePruneResult.spaceReclaimedBytes) })}</p> : null}
              {cachePruneFailed ? <div className="inline-error" role="alert">{t("images.cacheClearFailed")}</div> : null}
            </fieldset>
            {pullFailed ? <div className="inline-error full" role="alert">{t("images.pullFailed")}</div> : null}

            <div className="form-section full"><h3>{t("environments.identity")}</h3></div>
            <label className="field"><span className="field-label">{t("environments.timezone")}<HelpTip label={t("environments.timezone")} text={t("environments.help.timezone")} /></span><FormSelect control={control} name="timezone" ariaLabel={t("environments.timezone")} options={[
              ...(!timezoneOptions.includes(watch("timezone") as typeof timezoneOptions[number]) ? [{ value: watch("timezone"), label: watch("timezone") }] : []),
              ...timezoneOptions.map((timezone) => ({ value: timezone, label: timezone })),
            ]} /></label>
            <label className="field"><span className="field-label">{t("environments.locale")}<HelpTip label={t("environments.locale")} text={t("environments.help.locale")} /></span><FormSelect control={control} name="locale" ariaLabel={t("environments.locale")} options={localeOptions.map((locale) => ({ value: locale, label: locale }))} /></label>

            <div className="form-section full"><h3>{t("environments.resources")}</h3></div>
            <label className="field"><span className="field-label">{t("environments.cpu")}<HelpTip label={t("environments.cpu")} text={t("environments.help.cpu")} /></span><FormSelect control={control} name="cpuLimit" ariaLabel={t("environments.cpu")} options={[{ value: "unlimited", label: t("environments.unlimited") }, ...[1, 2, 4, 8, 16, 32, 64].map((value) => ({ value: String(value), label: String(value) }))]} /></label>
            <label className="field"><span className="field-label">{t("environments.memory")}<HelpTip label={t("environments.memory")} text={t("environments.help.memory")} /></span><FormSelect control={control} name="memoryGiB" ariaLabel={t("environments.memory")} options={[{ value: "unlimited", label: t("environments.unlimited") }, ...[1, 2, 4, 8, 16, 32, 64, 128, 256].map((value) => ({ value: String(value), label: `${value} GiB` }))]} /></label>
            <label className="field full"><span className="field-label">{t("environments.shm")}<HelpTip label={t("environments.shm")} text={t("environments.help.shm")} /></span><FormSelect control={control} name="shmGiB" ariaLabel={t("environments.shm")} options={["0.5", "1", "2", "4", "8"].map((value) => ({ value, label: `${value} GiB` }))} /></label>

            <div className="form-section full"><h3>{t("environments.display")}</h3></div>
            <label className="field"><span className="field-label">{t("environments.resolution")}<HelpTip label={t("environments.resolution")} text={t("environments.help.resolution")} /></span><FormSelect control={control} name="resolution" ariaLabel={t("environments.resolution")} options={[{ value: "auto", label: t("environments.automatic") }, ...[[1280, 720], [1366, 768], [1600, 900], [1920, 1080], [2560, 1440], [3840, 2160]].map(([width, height]) => ({ value: `${width}x${height}`, label: `${width} × ${height}` }))]} /></label>
            <label className="field"><span className="field-label">{t("environments.wayland")}<HelpTip label={t("environments.wayland")} text={t("environments.help.wayland")} /></span><FormSelect control={control} name="wayland" ariaLabel={t("environments.wayland")} disabled={selectedImage?.waylandOnly} options={[{ value: "auto", label: t("environments.automatic") }, { value: "true", label: t("common.enabled") }, { value: "false", label: t("common.disabled") }]} /></label>
            <label className="field"><span className="field-label">{t("environments.gpu")}<HelpTip label={t("environments.gpu")} text={t("environments.help.gpu")} /></span><FormSelect control={control} name="gpu" ariaLabel={t("environments.gpu")} options={[{ value: "disabled", label: t("common.disabled") }, { value: "dri", label: "Intel / AMD DRI" }, { value: "nvidia", label: "NVIDIA" }]} /></label>
            <label className="field"><span className="field-label">{t("environments.audio")}<HelpTip label={t("environments.audio")} text={t("environments.help.audio")} /></span><FormSelect control={control} name="audio" ariaLabel={t("environments.audio")} options={[{ value: "true", label: t("common.enabled") }, { value: "false", label: t("common.disabled") }]} /></label>
            <label className="field"><span className="field-label">{t("environments.clipboard")}<HelpTip label={t("environments.clipboard")} text={t("environments.help.clipboard")} /></span><FormSelect control={control} name="clipboard" ariaLabel={t("environments.clipboard")} options={[{ value: "true", label: t("common.enabled") }, { value: "false", label: t("common.disabled") }]} /></label>
            <label className="field"><span className="field-label">{t("environments.fileTransfer")}<HelpTip label={t("environments.fileTransfer")} text={t("environments.help.fileTransfer")} /></span><FormSelect control={control} name="fileTransfer" ariaLabel={t("environments.fileTransfer")} options={[{ value: "upload,download", label: t("environments.transferBoth") }, { value: "upload", label: t("environments.transferUpload") }, { value: "download", label: t("environments.transferDownload") }, { value: "none", label: t("common.disabled") }]} /></label>

            <div className="form-section full"><h3>{t("environments.securitySettings")}</h3></div>
            <label className="field full danger-option"><span className="field-label"><strong>{t("environments.publication")}</strong><HelpTip label={t("environments.publication")} text={t("environments.help.publication")} /></span><FormSelect control={control} name="publication" ariaLabel={t("environments.publication")} disabled={!publicationAvailable} options={[{ value: "false", label: t("common.disabled") }, { value: "true", label: t("common.enabled") }]} /></label>
            {!publicationAvailable ? <div className="warning full" role="status">{t("environments.publicationNeedsSettings")}</div> : null}
            {publication === "true" ? <div className="warning full" role="alert">{t("environments.publicationWarning")}</div> : null}
            <label className="field full danger-option"><span className="field-label"><strong>{t("environments.dockerSocket")}</strong><HelpTip label={t("environments.dockerSocket")} text={t("environments.help.dockerSocket")} /></span><FormSelect control={control} name="dockerSocket" ariaLabel={t("environments.dockerSocket")} options={[{ value: "false", label: t("common.disabled") }, { value: "true", label: t("common.enabled") }]} /></label>
            {dockerSocket === "true" ? <div className="warning full" role="alert">{t("environments.dockerSocketWarning")}</div> : null}
            <label className="field full danger-option"><span className="field-label"><strong>{t("environments.seccomp")}</strong><HelpTip label={t("environments.seccomp")} text={t("environments.help.seccomp")} /></span><FormSelect control={control} name="seccomp" ariaLabel={t("environments.seccomp")} options={[{ value: "default", label: t("environments.seccompDefault") }, { value: "unconfined", label: t("environments.seccompUnconfined") }]} /></label>
            {seccomp === "unconfined" ? <div className="warning full" role="alert">{t("environments.seccompWarning")}</div> : null}

            <details className="advanced-section full">
              <summary>{t("environments.mounts")} <span className="count-badge">{mounts.fields.length}</span></summary>
              <p className="muted">{t("environments.mountsHint")}</p>
              <div className="repeater-list">{mounts.fields.map((field, index) => <div className="repeater-row mount-row" key={field.id}>
                <label className="field"><span>{t("environments.hostPath")}</span><input {...register(`mounts.${index}.hostPath`)} placeholder="/home/user/data" aria-invalid={!!errors.mounts?.[index]?.hostPath} /></label>
                <label className="field"><span>{t("environments.containerPath")}</span><input {...register(`mounts.${index}.containerPath`)} placeholder="/data" aria-invalid={!!errors.mounts?.[index]?.containerPath} /></label>
                <label className="field"><span>{t("environments.accessMode")}</span><FormSelect control={control} name={`mounts.${index}.readOnly`} ariaLabel={t("environments.accessMode")} options={[{ value: "true", label: t("environments.readOnly") }, { value: "false", label: t("environments.readWrite") }]} /></label>
                <button type="button" className="icon-button remove-row" onClick={() => mounts.remove(index)} aria-label={t("common.remove")}>×</button>
              </div>)}</div>
              <button type="button" className="secondary-button small-button" onClick={() => mounts.append({ hostPath: "", containerPath: "", readOnly: "true" })}>＋ {t("environments.addMount")}</button>
            </details>

            <details className="advanced-section full">
              <summary>{t("environments.officialAdvanced")} <span className="count-badge">{options.fields.length}</span></summary>
              <p className="muted">{t("environments.officialAdvancedHint", { count: officialWebtopOptions.length })}</p>
              <div className="repeater-list">{options.fields.map((field, index) => {
                const selectedKey = selectedOptions[index]?.key ?? field.key;
                const definition = getOfficialOption(selectedKey) ?? officialWebtopOptions[0];
                const valueOptions = valuesForOption(definition);
                return <div className="repeater-row option-row" key={field.id}>
                  <label className="field"><span>{t("environments.officialParameter")}</span><FormSelect
                    control={control}
                    name={`officialOptions.${index}.key`}
                    ariaLabel={t("environments.officialParameter")}
                    options={officialOptionGroups.flatMap((group) => officialWebtopOptions
                      .filter((option) => option.group === group && (option.key === selectedKey || !selectedOptions.some((selected) => selected.key === option.key)))
                      .map((option) => ({ value: option.key, label: option.key, group: t(`environments.optionGroups.${group}`) })))}
                    onValueChange={(value) => {
                      const nextDefinition = getOfficialOption(value);
                      if (nextDefinition) setValue(`officialOptions.${index}.value`, nextDefinition.defaultValue, { shouldValidate: true });
                    }}
                  /></label>
                  <label className="field"><span className="field-label">{t("environments.value")}<HelpTip label={definition.key} text={t(`webtopOptions.${definition.key}`)} /></span>{valueOptions ? <FormSelect control={control} name={`officialOptions.${index}.value`} ariaLabel={t("environments.value")} options={valueOptions.map((value) => ({ value, label: t(`environments.optionValues.${value}`, { defaultValue: value }) }))} /> : <input type={definition.kind === "number" ? "number" : "text"} {...register(`officialOptions.${index}.value`)} placeholder={definition.placeholder} spellCheck={false} aria-invalid={!!errors.officialOptions?.[index]?.value} />}</label>
                  <button type="button" className="icon-button remove-row" onClick={() => options.remove(index)} aria-label={t("common.remove")}>×</button>
                </div>;
              })}</div>
              <button type="button" className="secondary-button small-button" disabled={options.fields.length === officialWebtopOptions.length} onClick={appendOfficialOption}>＋ {t("environments.addOfficialOption")}</button>
              <p className="security-note">{t("environments.managedConfigHint")}</p>
            </details>
          </div>
          <footer className="dialog-actions"><button type="button" className="secondary-button" disabled={pending} onClick={onClose}>{t("common.cancel")}</button><button type="submit" className={pending ? "primary-button is-working" : "primary-button"} disabled={pending || imagesLoading || !selectionReady}>{t(pending ? "environments.creating" : "common.create")}</button></footer>
        </form>
      </section>
    </div>
  );
}
