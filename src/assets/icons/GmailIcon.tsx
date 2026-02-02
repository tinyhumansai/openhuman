const GmailIcon = ({ className = 'w-4 h-4' }: { className?: string }) => {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="currentColor">
      <path d="M24 5.5v13.05c0 .85-.73 1.59-1.59 1.59H1.59C.73 21.14 0 20.4 0 19.55V5.5L12 13.25 24 5.5zM24 4.5c0-.42-.2-.83-.53-1.09L12 11.25.53 3.41C.2 3.67 0 4.08 0 4.5v.75L12 13 24 5.25V4.5z" />
      <path d="M5.5 4.5L12 9.75 18.5 4.5H5.5z" opacity="0.3" />
    </svg>
  );
};

export default GmailIcon;
