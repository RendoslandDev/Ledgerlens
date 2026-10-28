export function LogoMark({ size = 22 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
      className="logo-mark"
    >
      <rect x="3" y="2.5" width="14" height="19" rx="2.5" stroke="currentColor" strokeWidth="1.8" />
      <path d="M6.5 7.5h7M6.5 11h5" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
      <circle cx="15.5" cy="15.5" r="3.6" fill="var(--bg-0)" stroke="var(--accent)" strokeWidth="1.8" />
      <path d="m18.2 18.2 2.6 2.6" stroke="var(--accent)" strokeWidth="1.8" strokeLinecap="round" />
    </svg>
  );
}
