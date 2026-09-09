import {createRoot, type Root} from "react-dom/client";

let brandRoot: Root | undefined;

/** Uses the shipped favicon so the collapsed navigation mark and browser tab share one identity. */
export function mountReiconBrand(target: Element | null): void {
    if (!(target instanceof HTMLElement)) return;
    brandRoot?.unmount();
    target.replaceChildren();
    brandRoot = createRoot(target);
    brandRoot.render(<img className="brand-favicon" src="/favicon.svg" alt="" aria-hidden="true"/>);
}
