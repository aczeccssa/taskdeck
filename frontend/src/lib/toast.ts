let toastTimer: ReturnType<typeof setTimeout> | null = null;

/** Legacy showToast contract: #toast textContent + .visible class, cleared after 2200ms. */
export function showToast(message: string): void {
    const toast = document.getElementById("toast");
    if (!toast) return;
    toast.textContent = message;
    toast.classList.add("visible");
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
        toast.classList.remove("visible");
        toast.textContent = "";
    }, 2200);
}
