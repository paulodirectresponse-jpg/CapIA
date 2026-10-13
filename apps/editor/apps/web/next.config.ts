import type { NextConfig } from "next";

// CapIA: local-first build. No auth/DB/CMS/analytics plugins, no server-side
// image optimizer (sharp/libvips LGPL removed), no remote font download at build.
const nextConfig: NextConfig = {
	compiler: {
		removeConsole: process.env.NODE_ENV === "production",
	},
	reactStrictMode: true,
	productionBrowserSourceMaps: true,
	output: "standalone",
	images: { unoptimized: true },
};

export default nextConfig;
