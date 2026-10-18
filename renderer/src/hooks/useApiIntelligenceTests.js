import { useRef, useState } from 'react';
import { getErrorMessage } from '../utils/errors';
import { INTELLIGENCE_TESTS, gradeIntelligenceResponse } from '../utils/apiIntelligence';

// The 降智 test runs of each API profile in this session. Every question is sent at once and
// shows up as soon as its answer arrives; a new run replaces the previous one. Not saved.
export function useApiIntelligenceTests() {
  const [runs, setRuns] = useState({});
  const runIdsRef = useRef({});

  const runIntelligenceTests = (profile, profileId, model) => {
    const runId = (runIdsRef.current[profileId] || 0) + 1;
    runIdsRef.current[profileId] = runId;
    const pending = Object.fromEntries(INTELLIGENCE_TESTS.map(test => [test.id, { loading: true }]));
    setRuns(prev => ({ ...prev, [profileId]: { model, results: pending } }));

    INTELLIGENCE_TESTS.forEach(test => {
      window.api.testApiPrompt({
        baseUrl: profile.base_url || '',
        apiKey: profile.api_key || '',
        model,
        input: test.prompt,
        maxOutputTokens: test.maxOutputTokens
      })
        .then(response => ({ response, grade: gradeIntelligenceResponse(test, response) }))
        .catch(err => {
          console.error(`[test_api_prompt] ${test.id} failed`, err);
          return {
            response: null,
            grade: { status: 'error', answer: getErrorMessage(err, '测试题发送失败') }
          };
        })
        .then(result => {
          if (runIdsRef.current[profileId] !== runId) return;
          setRuns(prev => {
            const run = prev[profileId];
            return {
              ...prev,
              [profileId]: {
                ...run,
                results: { ...run.results, [test.id]: { loading: false, ...result } }
              }
            };
          });
        });
    });
  };

  return { intelligenceRuns: runs, runIntelligenceTests };
}
