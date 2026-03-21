export default function Footer() {
    return (
        <footer className="border-t border-[var(--border-subtle)] bg-[#0c0c0c]">
            <div className="w-full px-6 py-6 sm:px-8">
                <div className="flex flex-wrap items-center justify-center gap-4 text-sm text-zinc-400">
                    <span>© {new Date().getFullYear()} OpenHuman</span>
                    <span className="text-zinc-600">•</span>
                    <a
                        href="https://openhuman.xyz/privacy"
                        target="_blank"
                        rel="noopener noreferrer"
                        className="transition-colors hover:text-white"
                    >
                        Privacy Policy
                    </a>
                    <span className="text-zinc-600">•</span>
                    <a
                        href="https://openhuman.xyz/terms"
                        target="_blank"
                        rel="noopener noreferrer"
                        className="transition-colors hover:text-white"
                    >
                        Terms & Conditions
                    </a>
                </div>
            </div>
        </footer>
    );
}
