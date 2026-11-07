import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center whitespace-nowrap font-headline text-sm transition-all focus-visible:outline-none focus-visible:shadow-[rgba(0,0,0,0.1)_0px_4px_12px] disabled:pointer-events-none disabled:opacity-50 active:scale-[0.97]",
  {
    variants: {
      variant: {
        default:
          "bg-surface-300 text-cursor-dark hover:text-error-warm rounded-lg px-4 py-2.5 border border-[var(--fg-08)] hover:border-[rgba(207,45,86,0.2)] hover:shadow-[0_2px_8px_rgba(0,0,0,0.04)]",
        destructive: "bg-error-warm text-surface-200 hover:bg-error-warm/90 rounded-lg px-4 py-2.5",
        outline:
          "border border-[var(--fg-10)] bg-transparent hover:bg-surface-300 rounded-lg px-4 py-2.5 hover:border-[var(--fg-15)]",
        secondary:
          "bg-surface-400 text-[var(--fg-60)] hover:text-error-warm rounded-full px-3 py-1 text-xs",
        ghost:
          "text-[var(--fg-55)] hover:bg-[var(--fg-06)] hover:text-cursor-dark rounded-lg px-3 py-1.5",
        link: "text-cursor-orange underline-offset-4 hover:underline",
      },
      size: {
        default: "h-9",
        sm: "h-8 text-xs px-3 py-1.5",
        lg: "h-10 text-base px-5 py-2.5",
        icon: "h-9 w-9",
      },
    },
    defaultVariants: { variant: "default", size: "default" },
  },
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>, VariantProps<typeof buttonVariants> {}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, ...props }, ref) => (
    <button className={cn(buttonVariants({ variant, size, className }))} ref={ref} {...props} />
  ),
);
Button.displayName = "Button";

export { Button, buttonVariants };
