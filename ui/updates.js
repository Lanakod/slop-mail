// Only the main window owns the timer. Manual and scheduled checks share a request.
export function updateChecks({
  hours,
  fetchVersion,
  announce,
  storage,
  clock = window,
}) {
  let timer,
    pending,
    generation = 0;
  function check() {
    if (!pending) {
      storage.setItem("update-last-check", String(Date.now()));
      pending = Promise.resolve()
        .then(fetchVersion)
        .finally(() => {
          pending = null;
        });
    }
    return pending;
  }
  function schedule() {
    const current = ++generation;
    clock.clearTimeout(timer);
    const interval = hours() * 3600000;
    if (!interval) return;
    const elapsed = Math.max(
      0,
      Date.now() - Number(storage.getItem("update-last-check") || 0),
    );
    timer = clock.setTimeout(
      async () => {
        try {
          const version = await check();
          if (current === generation) await announce(version);
        } catch {
          // Offline checks resume at the configured interval.
        } finally {
          if (current === generation) schedule();
        }
      },
      Math.max(0, interval - elapsed),
    );
  }
  return { check, schedule };
}
