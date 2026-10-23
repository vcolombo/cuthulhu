// SPDX-License-Identifier: GPL-3.0-or-later
import type { ComponentProps } from "react";
import { Toaster as Sonner } from "sonner";
import "sonner/dist/styles.css";

type ToasterProps = ComponentProps<typeof Sonner>;

export function Toaster({ ...props }: ToasterProps) {
  return (
    <Sonner
      className="toaster group"
      toastOptions={{
        classNames: {
          toast: "group toast group-[.toaster]:bg-card group-[.toaster]:text-foreground group-[.toaster]:border-border group-[.toaster]:shadow-lg",
          description: "group-[.toast]:text-muted-foreground",
          actionButton: "group-[.toast]:bg-secondary group-[.toast]:text-foreground",
        },
      }}
      {...props}
    />
  );
}
