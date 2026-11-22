import type { Metadata } from "next";
import type { ReactNode } from "react";

export const metadata: Metadata = {
  title: "SYNAPSE Studio",
  description: "Design once. Simulate in 3D. Compile to any machine.",
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body
        style={{
          margin: 0,
          fontFamily: "system-ui, sans-serif",
          background: "#0b0d10",
          color: "#e6e8eb",
        }}
      >
        {children}
      </body>
    </html>
  );
}
