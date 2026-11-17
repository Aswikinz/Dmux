import { clsx } from 'clsx';
import { twMerge } from 'tailwind-merge';

/** Merge Tailwind classes without conflicts. */
export function cn(...inputs: (string | undefined | false | null)[]) {
  return twMerge(clsx(inputs));
}
