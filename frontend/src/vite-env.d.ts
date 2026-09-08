/// <reference types="vite/client" />
/// <reference types="bun-types" />

declare module "*.html?raw" {
    const content: string;
    export default content;
}

declare module "*.js";

declare module "*.js?raw" {
    const content: string;
    export default content;
}
