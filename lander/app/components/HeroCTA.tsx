'use client';

import { useState } from 'react';
import DownloadModal from './DownloadModal';

export default function HeroCTA() {
  const [downloadModalOpen, setDownloadModalOpen] = useState(false);

  return (
    <>
      <div className="mt-10 flex justify-center">
        <button
          type="button"
          onClick={() => setDownloadModalOpen(true)}
          className="rounded-lg bg-[var(--accent)] px-6 py-2.5 text-sm font-semibold text-white shadow-none transition-colors hover:bg-[var(--accent-hover)] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          Download
        </button>
      </div>
      <DownloadModal isOpen={downloadModalOpen} onClose={() => setDownloadModalOpen(false)} />
    </>
  );
}
