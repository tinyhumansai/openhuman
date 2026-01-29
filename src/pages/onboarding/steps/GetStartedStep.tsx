import { useState } from "react";
import ConnectionIndicator from "../../../components/ConnectionIndicator";

interface GetStartedStepProps {
  onComplete: () => void | Promise<void>;
}

const GetStartedStep = ({ onComplete }: GetStartedStepProps) => {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleOpenTelegram = async () => {
    setError(null);
    setLoading(true);
    try {
      await onComplete();
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "Something went wrong. Please try again.",
      );
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="glass rounded-3xl p-8 shadow-large animate-fade-up">
      <div className="text-center mb-4">
        <h1 className="text-xl font-bold mb-2">You Are Ready, Soldier!</h1>
        <p className="opacity-70 text-sm">
          Alright you're all set up, just message your assistant and you're
          ready to cook! Remember to keep this tab open to keep the connection
          alive.
        </p>
      </div>

      <ConnectionIndicator description="Your browser is now connected to the AlphaHuman AI Models. Please keep this tab open." />

      {error && (
        <p className="text-coral-500 text-sm mb-3 text-center">{error}</p>
      )}

      <button
        onClick={handleOpenTelegram}
        disabled={loading}
        className="w-full flex items-center justify-center space-x-3 bg-blue-500 hover:bg-blue-600 active:bg-blue-700 disabled:opacity-60 disabled:cursor-not-allowed text-white font-semibold py-2.5 text-sm rounded-xl transition-all duration-300 hover:shadow-medium mb-3"
      >
        <span>{loading ? "Finishing…" : "I'm Ready! Let's Go! 🔥"}</span>
      </button>
    </div>
  );
};

export default GetStartedStep;
