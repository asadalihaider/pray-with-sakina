import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export type Place = {
  name: string;
  admin1?: string;
  country?: string;
  latitude: number;
  longitude: number;
  timezone: string;
};

/// Open-Meteo's geocoding service needs no key and returns the IANA
/// timezone alongside the coordinates, which bedtime and jamaat times need.
const ENDPOINT = "https://geocoding-api.open-meteo.com/v1/search";

type Detected = {
  latitude: number;
  longitude: number;
  timezone: string;
  name: string | null;
};

export default function CitySearch({
  onPick,
  onBusy,
}: {
  onPick: (place: Place) => void;
  /// Reported upward because the screen around this one has to stop
  /// accepting Next and Skip while a fix is being waited for: leaving now
  /// saves a location that is about to be replaced.
  onBusy?: (busy: boolean) => void;
}) {
  const [query, setQuery] = useState("");
  const [places, setPlaces] = useState<Place[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [locating, setLocating] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  /// Named rather than assumed. Finding a location is the one thing on this
  /// screen that takes a visible moment and then changes something the user
  /// cannot see happening, so it says so when it is done.
  const [fetched, setFetched] = useState<string | null>(null);

  const useMyLocation = async () => {
    setLocating(true);
    onBusy?.(true);
    setProblem(null);
    setFetched(null);
    try {
      const found = await invoke<Detected>("detect_location");
      // Apple's geocoder names the coordinates it already holds; if it
      // cannot, a generic label beats blocking on a name.
      const name = found.name ?? "Current location";
      onPick({
        name,
        latitude: found.latitude,
        longitude: found.longitude,
        timezone: found.timezone,
      });
      setFetched(name);
    } catch (error) {
      setProblem(String(error));
    } finally {
      setLocating(false);
      onBusy?.(false);
    }
  };

  const search = async () => {
    if (query.trim().length < 2) return;
    setSearching(true);
    try {
      const response = await fetch(
        `${ENDPOINT}?count=5&language=en&format=json&name=${encodeURIComponent(
          query.trim()
        )}`
      );
      const body = await response.json();
      setPlaces(body.results ?? []);
    } catch {
      setPlaces([]);
    } finally {
      setSearching(false);
    }
  };

  const pick = (place: Place) => {
    setPlaces(null);
    setQuery("");
    onPick(place);
  };

  return (
    <>
      <div className="row">
        <input
          className="search"
          placeholder="Search a city"
          value={query}
          onChange={(event) => {
          setQuery(event.target.value);
          setFetched(null);
        }}
          onKeyDown={(event) => event.key === "Enter" && search()}
        />
        <button className="small" onClick={search} disabled={searching}>
          {searching ? "…" : "Find"}
        </button>
      </div>
      <div className="row">
        <button className="small wide" onClick={useMyLocation} disabled={locating}>
          {locating ? "Locating…" : "Use my location"}
        </button>
      </div>
      {problem && <div className="row-note">{problem}</div>}
      {fetched && !problem && (
        <div className="row-note is-good">Current location found — {fetched}</div>
      )}
      {places !== null && (
        <div className="places">
          {places.length === 0 && <div className="row-note">No matches.</div>}
          {places.map((place) => (
            <button
              key={`${place.latitude},${place.longitude}`}
              className="place"
              onClick={() => pick(place)}
            >
              {place.name}
              <span>{[place.admin1, place.country].filter(Boolean).join(", ")}</span>
            </button>
          ))}
        </div>
      )}
    </>
  );
}
