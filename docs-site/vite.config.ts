import {defineConfig} from "vite";
import react from "@vitejs/plugin-react";
export default defineConfig({plugins:[react()], base: process.env.BASE_PATH ?? "/taskdeck/", server:{allowedHosts:["windows-company-hp66"]}, build:{outDir:"dist",emptyOutDir:true}});
