export function formatClock(iso: string, timeZone: string) {
  return new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
    timeZone,
  }).format(new Date(iso));
}

export function formatDuration(ms: number) {
  const minutes = Math.max(0, Math.floor(ms / 60000));
  if (minutes >= 60) {
    const hours = Math.floor(minutes / 60);
    const rest = minutes % 60;
    return rest === 0 ? `${hours}h` : `${hours}h ${rest}m`;
  }
  return minutes >= 1 ? `${minutes}m` : "<1m";
}
