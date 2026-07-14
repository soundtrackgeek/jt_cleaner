export function formatBytes(bytes = 0, maximumFractionDigits = 1) {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  const unitIndex = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** unitIndex;
  return `${value.toLocaleString(undefined, {
    maximumFractionDigits: unitIndex === 0 ? 0 : maximumFractionDigits,
  })} ${units[unitIndex]}`;
}

export function getScanUsage(result) {
  const driveTotalBytes = result?.driveTotalBytes;
  const driveUsedBytes = result?.driveUsedBytes;
  if (Number.isFinite(driveTotalBytes) && driveTotalBytes > 0 && Number.isFinite(driveUsedBytes)) {
    return {
      usedBytes: Math.min(Math.max(driveUsedBytes, 0), driveTotalBytes),
      totalBytes: driveTotalBytes,
      isDrive: true,
    };
  }
  return { usedBytes: result?.totalBytes || 0, totalBytes: null, isDrive: false };
}

export function formatScanSize(result) {
  const usage = getScanUsage(result);
  return usage.isDrive
    ? `${formatBytes(usage.usedBytes)} used of ${formatBytes(usage.totalBytes)}`
    : formatBytes(usage.usedBytes);
}

export function formatScanProgressSize(progress) {
  return formatScanSize({
    totalBytes: progress?.scannedBytes || 0,
    driveTotalBytes: progress?.driveTotalBytes,
    driveUsedBytes: progress?.driveUsedBytes,
  });
}

export function formatCount(value = 0) {
  return new Intl.NumberFormat().format(value);
}

export function formatDuration(milliseconds = 0) {
  if (milliseconds < 1_000) return `${milliseconds} ms`;
  if (milliseconds < 60_000) return `${(milliseconds / 1_000).toFixed(1)} sec`;
  const minutes = Math.floor(milliseconds / 60_000);
  const seconds = Math.round((milliseconds % 60_000) / 1_000);
  return `${minutes} min ${seconds} sec`;
}

export function formatDateTime(value) {
  if (!value) return "Not available";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}
