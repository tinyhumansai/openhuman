import Link from 'next/link';
import HeroCTA from './components/HeroCTA';
import RotatingTetrahedronCanvas from './components/RotatingTetrahedronCanvas';

const connectLinks = [
  { label: 'X', href: 'https://x.com/tinyhumansai' },
  { label: 'GitHub', href: 'https://github.com/tinyhumansai/openhuman' },
];

export default function Home() {
  return (
    <div className="relative flex min-h-full flex-1 flex-col bg-[#121212] text-white">
      <main className="relative z-10 w-full">
        <section className="px-6 pb-12 pt-16 sm:px-10 sm:pb-16 sm:pt-20">
          <div className="mx-auto max-w-xl text-center">
            <p className="text-xs font-semibold uppercase tracking-[0.2em] text-zinc-500">OpenHuman</p>
            <div className="mx-auto mt-8 h-[min(200px,42vw)] w-[min(200px,42vw)] max-h-[220px] max-w-[220px]">
              <RotatingTetrahedronCanvas />
            </div>
            <h1 className="mt-10 text-4xl font-bold tracking-tight text-white sm:text-5xl">Get started</h1>
            <p className="mt-4 text-base leading-relaxed text-[var(--muted)] sm:text-lg">
              Your AI superhuman for personal and business life. Private beta — install the app and connect your
              tools.
            </p>
            <HeroCTA />
          </div>
        </section>

        <section className="border-t border-[var(--border-subtle)]">
          <div className="grid md:grid-cols-2">
            <div className="border-b border-[var(--border-subtle)] px-6 py-12 sm:px-10 md:border-b-0 md:border-r md:border-[var(--border-subtle)]">
              <h2 className="text-lg font-semibold tracking-tight text-white">Documentation</h2>
              <p className="mt-2 max-w-sm text-sm leading-relaxed text-zinc-400">
                Read the guides, learn how skills work, and explore what OpenHuman can do with your Telegram and
                integrations.
              </p>
              <div className="mt-8 flex flex-wrap gap-3">
                <a
                  href="https://tinyhumans.gitbook.io/openhuman"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex items-center justify-center rounded-full bg-[var(--surface-raised)] px-4 py-2 text-sm font-medium text-zinc-200 ring-1 ring-zinc-700/80 transition-colors hover:bg-zinc-800 hover:text-white"
                >
                  Read the docs
                </a>
                <Link
                  href="/downloads"
                  className="inline-flex items-center justify-center rounded-full bg-[var(--surface-raised)] px-4 py-2 text-sm font-medium text-zinc-200 ring-1 ring-zinc-700/80 transition-colors hover:bg-zinc-800 hover:text-white"
                >
                  View downloads
                </Link>
              </div>
            </div>
            <div className="px-6 py-12 sm:px-10">
              <h2 className="text-lg font-semibold tracking-tight text-white">Connect with us</h2>
              <p className="mt-2 max-w-sm text-sm leading-relaxed text-zinc-400">
                Follow updates, ship feedback, and join the community while we&apos;re in beta.
              </p>
              <div className="mt-8 flex flex-wrap gap-2">
                {connectLinks.map(({ label, href }) => (
                  <a
                    key={href}
                    href={href}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="inline-flex items-center justify-center rounded-full bg-[var(--surface-raised)] px-4 py-2 text-sm font-medium text-zinc-200 ring-1 ring-zinc-700/80 transition-colors hover:bg-zinc-800 hover:text-white"
                  >
                    {label}
                  </a>
                ))}
              </div>
            </div>
          </div>
        </section>
      </main>
    </div>
  );
}
