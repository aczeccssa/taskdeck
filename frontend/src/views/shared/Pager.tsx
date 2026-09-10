import type { Page } from "../../domain/models";

export function Pager({
    page,
    onPrev,
    onNext,
    prefix,
}: {
    page: Page;
    onPrev: () => void;
    onNext: () => void;
    prefix: string;
}): React.JSX.Element {
    return (
        <footer className="pager">
            <span id={`${prefix}-page-label`}>
                {page.total_pages ? `Page ${page.page} of ${page.total_pages}` : "Page 0 of 0"}
            </span>
            <div>
                <button className="button" type="button" disabled={!page.has_previous} onClick={onPrev}>
                    Previous
                </button>
                <button className="button" type="button" disabled={!page.has_next} onClick={onNext}>
                    Next
                </button>
            </div>
        </footer>
    );
}
